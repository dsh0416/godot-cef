//! CEF-independent editor extension. All Godot calls stay on the editor thread.
#![allow(clippy::redundant_field_names)] // GodotClass's generated initializer uses `base: base`.

mod export;

use gdcef_runtime::{InstallOptions, InstallSource, Manifest, Progress, host_target};
use godot::classes::{
    Button, CheckBox, ClassDb, DisplayServer, EditorExportPlugin, EditorInterface, EditorPlugin,
    FileDialog, HFlowContainer, IEditorPlugin, Label, MarginContainer, Os, ProgressBar,
    ProjectSettings, VBoxContainer, Window,
};
use godot::init::{ExtensionLibrary, InitStage, gdextension};
use godot::prelude::*;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

struct BootstrapLibrary;

#[derive(Clone)]
struct FinalActivation {
    addon: PathBuf,
    manifest: Manifest,
    extension_list: PathBuf,
    descriptor_resource_path: String,
    cli_install_outcome: Option<bool>,
}

static FINAL_ACTIVATION: Mutex<Option<FinalActivation>> = Mutex::new(None);

#[gdextension(entry_symbol = gdcef_bootstrap_init)]
unsafe impl ExtensionLibrary for BootstrapLibrary {
    fn on_stage_deinit(stage: InitStage) {
        if stage != InitStage::Scene {
            return;
        }
        // The editor and its file scanner have shut down. Publish the real core
        // descriptor now, so CEF is first loaded during the next Core startup.
        // Dynamic discovery followed by CLI exit races Godot 4.6's async docs:
        // https://github.com/godotengine/godot/issues/111048
        let activation = FINAL_ACTIVATION
            .lock()
            .ok()
            .and_then(|mut pending| pending.take());
        let Some(activation) = activation else { return };
        match gdcef_runtime::finalize_extension_activation(
            &activation.addon,
            &activation.manifest,
            &activation.extension_list,
            &activation.descriptor_resource_path,
        ) {
            Ok(()) if activation.cli_install_outcome == Some(true) => {
                println!("GODOT_CEF_INSTALL_OK")
            }
            Ok(()) => {}
            Err(error) => {
                eprintln!(
                    "[Godot CEF Setup] Cannot activate the installed runtime: {error}. Open setup and restart the editor to retry."
                );
                if activation.cli_install_outcome.is_some() {
                    // Explicit installer CLI contract: a failed final commit
                    // must fail the command, even after SceneTree has exited.
                    std::process::exit(1);
                }
            }
        }
    }
}

#[derive(Clone)]
enum Action {
    Open,
    Close,
    Install,
    Import,
    Archive(String),
    Cancel,
    Restart,
    SelectAll,
}

thread_local! {
    static ACTIONS: RefCell<Vec<Action>> = const { RefCell::new(Vec::new()) };
}

fn callback(name: &'static str, action: Action) -> Callable {
    Callable::from_fn(name, move |_| {
        ACTIONS.with(|queue| queue.borrow_mut().push(action.clone()));
    })
}

struct Job {
    cancel: Arc<AtomicBool>,
    progress: Receiver<Progress>,
    completion: Receiver<Result<(), String>>,
    worker: JoinHandle<()>,
}

#[derive(Default, Debug, PartialEq)]
struct CommandLine {
    targets: Option<String>,
    check: Option<String>,
    source: Option<PathBuf>,
}

impl CommandLine {
    fn parse(args: impl IntoIterator<Item = String>) -> Self {
        let mut result = Self::default();
        for arg in args {
            if let Some(targets) = arg.strip_prefix("--godot-cef-install=") {
                result.targets = Some(targets.to_owned());
            } else if let Some(target) = arg.strip_prefix("--godot-cef-check=") {
                result.check = Some(target.to_owned());
            } else if let Some(source) = arg.strip_prefix("--godot-cef-source=") {
                result.source = Some(PathBuf::from(source));
            }
        }
        result
    }
}

#[derive(GodotClass)]
#[class(tool, init, base=EditorPlugin)]
struct CefSetupPlugin {
    base: Base<EditorPlugin>,
    addon: Option<PathBuf>,
    manifest: Option<Manifest>,
    window: Option<Gd<Window>>,
    file_dialog: Option<Gd<FileDialog>>,
    status: Option<Gd<Label>>,
    progress_bar: Option<Gd<ProgressBar>>,
    selections: Vec<(String, Gd<CheckBox>)>,
    select_all_button: Option<Gd<Button>>,
    install_button: Option<Gd<Button>>,
    import_button: Option<Gd<Button>>,
    cancel_button: Option<Gd<Button>>,
    restart_button: Option<Gd<Button>>,
    export_plugin: Option<Gd<export::CefRuntimeExport>>,
    job: Option<Job>,
    first_frame: bool,
    startup_popup_pending: bool,
    startup_popup_frames: u8,
    cli: bool,
    restart_required: bool,
}

#[godot_api]
impl IEditorPlugin for CefSetupPlugin {
    fn enter_tree(&mut self) {
        self.first_frame = true;
        self.base_mut().set_process(true);
        self.addon = locate_addon();
        let Some(addon) = self.addon.as_ref() else {
            godot_warn!(
                "[Godot CEF Setup] runtime-install.json was not found beside the extension"
            );
            return;
        };
        match Manifest::load(addon) {
            Ok(manifest) => self.manifest = Some(manifest),
            Err(error) => {
                godot_error!("[Godot CEF Setup] {error}");
                return;
            }
        }
        // Capture the engine's actual cache directory before editor teardown.
        // This also recovers an installed runtime from an interrupted session.
        if let Some(manifest) = &self.manifest {
            match capture_activation(addon, manifest) {
                Ok(activation) => {
                    if let Ok(mut pending) = FINAL_ACTIVATION.lock() {
                        *pending = Some(activation);
                    }
                }
                Err(error) => {
                    godot_error!("[Godot CEF Setup] {error}");
                }
            }
            match gdcef_runtime::defer_extension_activation(addon, manifest) {
                Ok(true) => self.restart_required = true,
                Ok(false) => {}
                Err(error) => {
                    godot_warn!("[Godot CEF Setup] Cannot recover runtime activation: {error}");
                }
            }
        }
        let export = export::create(addon.clone());
        self.base_mut()
            .add_export_plugin(&export.clone().upcast::<EditorExportPlugin>());
        self.export_plugin = Some(export);
        self.base_mut().add_tool_menu_item(
            "Godot CEF Setup…",
            &callback("cef_setup_open", Action::Open),
        );
    }

    fn process(&mut self, _delta: f64) {
        if self.first_frame {
            self.first_frame = false;
            self.startup();
        }
        let actions = ACTIONS.with(|queue| std::mem::take(&mut *queue.borrow_mut()));
        for action in actions {
            self.handle_action(action);
        }
        self.poll_job();
        self.poll_startup_popup();
    }

    fn exit_tree(&mut self) {
        self.base_mut().remove_tool_menu_item("Godot CEF Setup…");
        if let Some(export) = self.export_plugin.take() {
            self.base_mut()
                .remove_export_plugin(&export.upcast::<EditorExportPlugin>());
        }
        // Native library code must not be unloaded while its worker is executing.
        if let Some(job) = self.job.take() {
            job.cancel.store(true, Ordering::Relaxed);
            let _ = job.worker.join();
        }
        if let Some(mut window) = self.window.take() {
            window.queue_free();
        }
        if let Some(mut dialog) = self.file_dialog.take() {
            dialog.queue_free();
        }
        ACTIONS.with(|queue| queue.borrow_mut().clear());
    }

    fn build(&mut self) -> bool {
        let (Some(addon), Some(manifest)) = (&self.addon, &self.manifest) else {
            return true;
        };
        let ready = self.job.is_none()
            && !self.restart_required
            && host_core_loaded(addon, manifest).is_ok();
        if !ready {
            godot_error!(
                "Godot CEF is not ready in this editor. Complete setup, then restart the editor."
            );
            self.open_window();
        }
        ready
    }
}

impl CefSetupPlugin {
    fn startup(&mut self) {
        let args = Os::singleton().get_cmdline_user_args();
        let command = CommandLine::parse(args.as_slice().iter().map(ToString::to_string));
        if let Some(target) = command.check {
            self.cli = true;
            let target = if target == "host" {
                host_target()
            } else {
                &target
            };
            let result = match (&self.addon, &self.manifest) {
                (Some(addon), Some(manifest)) => {
                    gdcef_runtime::prepare_export(addon, manifest, target)
                        .map_err(|error| error.to_string())
                }
                _ => Err("The installation manifest is missing or invalid.".to_owned()),
            };
            match result {
                Ok(()) => match (&self.addon, &self.manifest) {
                    (Some(addon), Some(manifest)) => match host_core_loaded(addon, manifest) {
                        Ok(()) => {
                            godot_print!("GODOT_CEF_CHECK_OK");
                            self.quit(0);
                        }
                        Err(error) => self.finish_error(&error),
                    },
                    _ => self.finish_error("The installation manifest is missing or invalid."),
                },
                Err(error) => self.finish_error(&error),
            }
        } else if let Some(targets) = command.targets {
            self.cli = true;
            record_cli_install(false);
            let selected = match targets.as_str() {
                "host" => vec![host_target().to_owned()],
                "all" => self
                    .manifest
                    .as_ref()
                    .map(|m| m.platforms.iter().map(|p| p.target.clone()).collect())
                    .unwrap_or_default(),
                _ => targets.split(',').map(str::to_owned).collect(),
            };
            self.start_install(selected, source_for(command.source));
        } else if DisplayServer::singleton().get_name() != "headless" {
            let missing = match (&self.addon, &self.manifest) {
                (Some(addon), Some(manifest)) => host_core_loaded(addon, manifest).is_err(),
                _ => false,
            };
            if missing {
                self.startup_popup_pending = true;
            }
        }
    }

    fn poll_startup_popup(&mut self) {
        if !self.startup_popup_pending {
            return;
        }
        let editor = EditorInterface::singleton();
        let ready = editor
            .get_resource_filesystem()
            .is_some_and(|filesystem| !filesystem.is_scanning())
            && editor.get_base_control().is_some_and(|control| {
                control.is_visible_in_tree()
                    && control
                        .get_window()
                        .is_some_and(|window| window.is_visible())
            });
        if !ready {
            self.startup_popup_frames = 0;
            return;
        }
        // A fresh Asset Library import can create the plugin while Godot is
        // still showing its loading screen and restoring the editor layout.
        self.startup_popup_frames += 1;
        if self.startup_popup_frames < 3 {
            return;
        }
        self.startup_popup_pending = false;
        self.open_window();
    }

    fn open_window(&mut self) {
        self.startup_popup_pending = false;
        if self.window.is_none() {
            self.create_window();
        }
        if let Some(window) = self.window.as_mut() {
            window.popup_centered();
        }
    }

    fn create_window(&mut self) {
        let Some(manifest) = self.manifest.as_ref() else {
            return;
        };
        let title = format!(
            "Godot CEF {} · Chromium Embedded Framework {}",
            manifest.plugin_version, manifest.cef_version
        );
        let platforms: Vec<_> = manifest
            .platforms
            .iter()
            .map(|platform| platform.target.clone())
            .collect();
        // Editor theme fonts already follow this scale; custom window geometry
        // and spacing must use the same units on high-DPI displays.
        let scale = EditorInterface::singleton().get_editor_scale().max(0.1);
        let px = |logical: f32| (logical * scale).round() as i32;
        let mut window = Window::new_alloc();
        window.set_title("Godot CEF Setup");
        window.set_transient(true);
        window.set_size(Vector2i::new(px(760.0), px(560.0)));
        window.set_min_size(Vector2i::new(px(620.0), px(460.0)));
        window.connect(
            "close_requested",
            &callback("cef_setup_close", Action::Close),
        );
        let mut margin = MarginContainer::new_alloc();
        margin.set_anchors_and_offsets_preset(godot::classes::control::LayoutPreset::FULL_RECT);
        for side in ["margin_left", "margin_top", "margin_right", "margin_bottom"] {
            margin.add_theme_constant_override(side, px(20.0));
        }
        let mut column = VBoxContainer::new_alloc();
        column.add_theme_constant_override("separation", px(12.0));
        let mut heading = Label::new_alloc();
        heading.set_text(&title);
        heading.set_autowrap_mode(godot::classes::text_server::AutowrapMode::WORD_SMART);
        column.add_child(&heading);
        let mut description = Label::new_alloc();
        description.set_text("Download browser components for any supported platform from this computer. Select individual platforms or download them all. Install this computer's platform to use CEF in the editor.");
        description.set_autowrap_mode(godot::classes::text_server::AutowrapMode::WORD_SMART);
        column.add_child(&description);
        for target in platforms {
            let mut selection = CheckBox::new_alloc();
            selection.set_text(platform_name(&target));
            selection.set_pressed(target == host_target());
            column.add_child(&selection);
            self.selections.push((target, selection));
        }
        let mut actions = HFlowContainer::new_alloc();
        actions.add_theme_constant_override("h_separation", px(8.0));
        actions.add_theme_constant_override("v_separation", px(8.0));
        let mut all = button("Select all platforms", "cef_setup_all", Action::SelectAll);
        actions.add_child(&all);
        all.set_tooltip_text("Download complete browser components for every supported platform.");
        self.select_all_button = Some(all);
        let install = button("Install selected", "cef_setup_install", Action::Install);
        actions.add_child(&install);
        self.install_button = Some(install);
        let import = button("Import CEF archive…", "cef_setup_import", Action::Import);
        actions.add_child(&import);
        self.import_button = Some(import);
        column.add_child(&actions);
        let mut progress = ProgressBar::new_alloc();
        progress.set_show_percentage(false);
        column.add_child(&progress);
        self.progress_bar = Some(progress);
        let mut status = Label::new_alloc();
        status.set_autowrap_mode(godot::classes::text_server::AutowrapMode::WORD_SMART);
        status.set_v_size_flags(godot::classes::control::SizeFlags::EXPAND_FILL);
        column.add_child(&status);
        self.status = Some(status);
        let mut footer = HFlowContainer::new_alloc();
        footer.add_theme_constant_override("h_separation", px(8.0));
        footer.add_theme_constant_override("v_separation", px(8.0));
        let mut cancel = button("Cancel download", "cef_setup_cancel", Action::Cancel);
        cancel.set_disabled(true);
        footer.add_child(&cancel);
        self.cancel_button = Some(cancel);
        let mut restart = button(
            "Save and restart editor",
            "cef_setup_restart",
            Action::Restart,
        );
        restart.set_disabled(true);
        footer.add_child(&restart);
        self.restart_button = Some(restart);
        column.add_child(&footer);
        margin.add_child(&column);
        window.add_child(&margin);
        self.base_mut().add_child(&window);
        self.window = Some(window);
        self.refresh_status();
        self.set_busy(self.job.is_some());
    }

    fn handle_action(&mut self, action: Action) {
        match action {
            Action::Open => self.open_window(),
            Action::Close => {
                if let Some(window) = self.window.as_mut() {
                    window.hide();
                }
            }
            Action::Install => self.start_install(self.selected_targets(), InstallSource::Download),
            Action::Import => self.import_archive(),
            Action::Archive(path) => self.start_install(
                self.selected_targets(),
                InstallSource::LocalArchive(path.into()),
            ),
            Action::Cancel => {
                if let Some(job) = &self.job {
                    job.cancel.store(true, Ordering::Relaxed);
                    self.set_status("Cancelling installation…");
                }
            }
            Action::Restart => {
                EditorInterface::singleton().restart_editor();
            }
            Action::SelectAll => {
                for (_, selection) in &mut self.selections {
                    selection.set_pressed(true);
                }
            }
        }
    }

    fn import_archive(&mut self) {
        let selected = self.selected_targets();
        if selected
            .iter()
            .any(|target| target == "universal-apple-darwin")
        {
            self.set_status("macOS needs both ARM64 and x86_64 CEF packages. Use Install selected on any OS, or provide both extracted runtime directories with --godot-cef-install=universal-apple-darwin --godot-cef-source=<directory>. Final signing runs on macOS.");
            return;
        }
        if selected.len() != 1 {
            self.set_status("Select one platform before importing its CEF archive.");
            return;
        }
        if self.file_dialog.is_none() {
            let mut dialog = FileDialog::new_alloc();
            dialog.set_title("Import a matching CEF archive");
            dialog.set_access(godot::classes::file_dialog::Access::FILESYSTEM);
            dialog.set_file_mode(godot::classes::file_dialog::FileMode::OPEN_FILE);
            dialog.add_filter("*.tar.bz2");
            dialog.connect(
                "file_selected",
                &Callable::from_fn("cef_setup_archive", |args| {
                    if let Some(path) = args.first().and_then(|v| v.try_to::<GString>().ok()) {
                        ACTIONS.with(|queue| {
                            queue.borrow_mut().push(Action::Archive(path.to_string()))
                        });
                    }
                }),
            );
            self.base_mut().add_child(&dialog);
            self.file_dialog = Some(dialog);
        }
        if let Some(dialog) = self.file_dialog.as_mut() {
            dialog.popup_centered_ratio();
        }
    }

    fn selected_targets(&self) -> Vec<String> {
        self.selections
            .iter()
            .filter(|(_, checkbox)| checkbox.is_pressed())
            .map(|(target, _)| target.clone())
            .collect()
    }

    fn start_install(&mut self, targets: Vec<String>, source: InstallSource) {
        if self.job.is_some() {
            return;
        }
        if targets.is_empty() {
            self.finish_error("Select at least one platform.");
            return;
        }
        let (Some(addon), Some(manifest)) = (&self.addon, &self.manifest) else {
            self.finish_error("The installation manifest is missing.");
            return;
        };
        let addon = addon.clone();
        let manifest = manifest.clone();
        let activation = FINAL_ACTIVATION
            .lock()
            .ok()
            .and_then(|pending| pending.clone());
        let Some(activation) = activation else {
            self.finish_error("The editor's extension cache path is unavailable. Restart the editor before installing CEF.");
            return;
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (progress_tx, progress_rx) = mpsc::channel();
        let (completion_tx, completion_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let options = InstallOptions {
                source,
                ..Default::default()
            };
            let result = gdcef_runtime::install(
                &addon,
                &manifest,
                &targets,
                &options,
                worker_cancel,
                progress_tx,
            )
            .and_then(|()| {
                gdcef_runtime::preflight_extension_activation(
                    &addon,
                    &manifest,
                    &activation.extension_list,
                    &activation.descriptor_resource_path,
                )
            })
            .map_err(|error| error.to_string());
            let _ = completion_tx.send(result);
        });
        self.job = Some(Job {
            cancel,
            progress: progress_rx,
            completion: completion_rx,
            worker,
        });
        self.set_busy(true);
        self.set_status("Preparing browser components…");
    }

    fn poll_job(&mut self) {
        let Some(job) = &self.job else { return };
        let updates: Vec<_> = job.progress.try_iter().collect();
        let completion = match job.completion.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(
                "The installation worker stopped unexpectedly.".to_owned(),
            )),
            Err(mpsc::TryRecvError::Empty) => None,
        };
        for update in updates {
            let detail = format!("{}: {}", platform_name(&update.target), update.message);
            self.set_status(&detail);
            if let Some(bar) = self.progress_bar.as_mut() {
                bar.set_value(
                    update
                        .total
                        .filter(|total| *total > 0)
                        .map_or(0.0, |total| update.downloaded as f64 * 100.0 / total as f64),
                );
            }
            if self.cli {
                godot_print!("[Godot CEF Setup] {detail}");
            }
        }
        if let Some(result) = completion {
            if let Some(job) = self.job.take() {
                let _ = job.worker.join();
            }
            if let (Some(addon), Some(manifest)) = (&self.addon, &self.manifest) {
                self.restart_required |=
                    gdcef_runtime::defer_extension_activation(addon, manifest).unwrap_or(false);
            }
            self.refresh_status();
            self.set_busy(false);
            match result {
                Ok(()) => {
                    if self.cli {
                        record_cli_install(true);
                    }
                    self.installation_ready();
                }
                Err(mut message) => {
                    if self.restart_required {
                        message.push_str("\nSome components were installed. Save and restart the editor to use them.");
                    }
                    self.finish_error(&message);
                }
            }
        }
    }

    fn installation_ready(&mut self) {
        self.set_busy(false);
        let statuses = self
            .addon
            .as_ref()
            .zip(self.manifest.as_ref())
            .map_or_else(Vec::new, |(addon, manifest)| {
                gdcef_runtime::statuses(addon, manifest)
            });
        let host_installed = statuses
            .iter()
            .any(|status| status.target == host_target() && status.installed);
        let mut message = if self.restart_required {
            "Installation complete. Save and restart the editor to load the browser extension."
        } else if !host_installed {
            "Selected platform downloads are complete. Install this computer's platform to use CEF in the editor."
        } else {
            "Selected platform components are installed."
        }
        .to_owned();
        if statuses.iter().any(|status| status.needs_signing) {
            message.push_str(
                " macOS components are downloaded; open setup on a Mac to finish signing.",
            );
        }
        self.set_status(&message);
        if let Some(button) = self.restart_button.as_mut() {
            button.set_disabled(!self.restart_required);
        }
        if let Some(bar) = self.progress_bar.as_mut() {
            bar.set_value(100.0);
        }
        if self.cli {
            self.quit(0);
        }
    }

    fn refresh_status(&mut self) {
        let (Some(addon), Some(manifest)) = (&self.addon, &self.manifest) else {
            return;
        };
        let statuses = gdcef_runtime::statuses(addon, manifest);
        for (target, checkbox) in &mut self.selections {
            let suffix = statuses
                .iter()
                .find(|status| status.target == *target)
                .map_or("Not installed", |status| {
                    if status.installed {
                        "Installed"
                    } else if status.needs_signing {
                        "Ready for signing on macOS"
                    } else {
                        "Not installed"
                    }
                });
            checkbox.set_text(&format!("{} — {}", platform_name(target), suffix));
        }
        if self.restart_required {
            self.set_status("The installed runtime is ready. Save and restart the editor to load the browser extension.");
        } else {
            self.set_status("Install the selected platforms, or import a matching CEF archive for an offline installation. Installed components remain available without a network connection.");
        }
    }

    fn set_busy(&mut self, busy: bool) {
        for (_, checkbox) in &mut self.selections {
            checkbox.set_disabled(busy);
        }
        for button in [
            &mut self.select_all_button,
            &mut self.install_button,
            &mut self.import_button,
        ]
        .into_iter()
        .flatten()
        {
            button.set_disabled(busy);
        }
        if let Some(button) = self.restart_button.as_mut() {
            button.set_disabled(busy || !self.restart_required);
        }
        if let Some(button) = self.cancel_button.as_mut() {
            button.set_disabled(self.job.is_none());
        }
    }

    fn set_status(&mut self, message: &str) {
        if let Some(label) = self.status.as_mut() {
            label.set_text(message);
        }
    }

    fn finish_error(&mut self, message: &str) {
        self.set_status(message);
        godot_error!("[Godot CEF Setup] {message}");
        if self.cli {
            self.quit(1);
        }
    }

    fn quit(&mut self, code: i32) {
        self.base().get_tree().quit_ex().exit_code(code).done();
    }
}

/// A previous version may still own CefTexture after a Store upgrade. Check the
/// loaded core's identity, not just the existence of its registered classes.
pub(crate) fn host_core_loaded(addon: &Path, manifest: &Manifest) -> Result<(), String> {
    let platform = manifest
        .platform(host_target())
        .map_err(|error| error.to_string())?;
    if !gdcef_runtime::statuses(addon, manifest)
        .iter()
        .any(|state| state.target == host_target() && state.installed)
    {
        return Err("Install the runtime for this computer and restart the editor before running or exporting the project.".to_owned());
    }
    let mut classes = ClassDb::singleton();
    if !classes.class_exists("CefTexture")
        || !classes.class_has_method("CefTexture", "get_runtime_library_path")
    {
        return Err("The current browser extension is not loaded. Save and restart the editor before running or exporting the project.".to_owned());
    }
    let loaded = classes
        .class_call_static("CefTexture", "get_runtime_library_path", &[])
        .try_to::<GString>()
        .map_err(|_| {
            "Cannot identify the loaded browser extension. Restart the editor.".to_owned()
        })?;
    if loaded.is_empty() {
        return Err("The browser extension was discovered after editor startup. Save and restart the editor before running or exporting the project.".to_owned());
    }
    let expected = addon
        .join(&platform.library)
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let actual = PathBuf::from(
        ProjectSettings::singleton()
            .globalize_path(&loaded)
            .to_string(),
    )
    .canonicalize()
    .map_err(|error| error.to_string())?;
    if actual != expected {
        return Err("A previous browser extension is still loaded. Save and restart the editor to use the installed version.".to_owned());
    }
    Ok(())
}

fn capture_activation(addon: &Path, manifest: &Manifest) -> Result<FinalActivation, String> {
    let paths = EditorInterface::singleton()
        .get_editor_paths()
        .ok_or("The editor's project cache directory is unavailable")?;
    let project = ProjectSettings::singleton();
    let settings_dir = PathBuf::from(
        project
            .globalize_path(&paths.get_project_settings_dir())
            .to_string(),
    );
    let data_dir = settings_dir
        .parent()
        .ok_or("The editor project cache has no parent directory")?;
    Ok(FinalActivation {
        addon: addon.to_owned(),
        manifest: manifest.clone(),
        extension_list: data_dir.join("extension_list.cfg"),
        descriptor_resource_path: project
            .localize_path(
                &addon
                    .join("godot_cef.gdextension")
                    .to_string_lossy()
                    .replace('\\', "/"),
            )
            .to_string(),
        cli_install_outcome: None,
    })
}

fn record_cli_install(succeeded: bool) {
    if let Ok(mut pending) = FINAL_ACTIVATION.lock()
        && let Some(activation) = pending.as_mut()
    {
        activation.cli_install_outcome = Some(succeeded);
    }
}

fn button(label: &str, signal: &'static str, action: Action) -> Gd<Button> {
    let mut result = Button::new_alloc();
    result.set_text(label);
    result.connect("pressed", &callback(signal, action));
    result
}

fn locate_addon() -> Option<PathBuf> {
    let library = process_path::get_dylib_path()?;
    library
        .ancestors()
        .find(|parent| parent.join("runtime-install.json").is_file())
        .map(Path::to_path_buf)
}

fn source_for(path: Option<PathBuf>) -> InstallSource {
    match path {
        Some(path) if path.is_dir() => InstallSource::LocalDirectory(path),
        Some(path) => InstallSource::LocalArchive(path),
        None => InstallSource::Download,
    }
}

fn platform_name(target: &str) -> &str {
    match target {
        "x86_64-pc-windows-msvc" => "Windows · x86_64",
        "aarch64-pc-windows-msvc" => "Windows · ARM64",
        "x86_64-unknown-linux-gnu" => "Linux · x86_64",
        "aarch64-unknown-linux-gnu" => "Linux · ARM64",
        "universal-apple-darwin" => "macOS · Universal",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::CommandLine;
    use std::path::PathBuf;

    #[test]
    fn cli_requires_explicit_install_and_preserves_source_spaces() {
        let command = CommandLine::parse([
            "--godot-cef-install=host".to_owned(),
            "--godot-cef-source=C:/CEF cache/runtime".to_owned(),
        ]);
        assert_eq!(command.targets.as_deref(), Some("host"));
        assert_eq!(command.source, Some(PathBuf::from("C:/CEF cache/runtime")));
        assert_eq!(
            CommandLine::parse(["--headless".to_owned()]),
            CommandLine::default()
        );
    }
}
