//! Runtime preflight for Godot's native exporter.
//!
//! Godot 4.6 does not expose an abort operation on EditorExportPlugin. Export
//! option warnings and export messages alone do not fail command-line export.
//! Missing real native dependencies are also submitted to the platform copier,
//! which fails normal executable exports. CI and PCK-only workflows must run
//! the explicit `--godot-cef-check=<target>` preflight before exporting.
//!
//! The bootstrap stays in the exported extension list, but its EditorPlugin
//! never runs in a game. Skipping only its descriptor is incorrect in 4.6:
//! Godot filters extension_list.cfg against selected paths, not plugin skips.

use gdcef_runtime::{Manifest, Platform};
use godot::builtin::{AnyDictionary, VariantType};
use godot::classes::editor_export_platform::ExportMessageType;
use godot::classes::{
    EditorExportPlatform, EditorExportPlugin, IEditorExportPlugin, Node, ProjectSettings, Resource,
};
use godot::prelude::*;
use godot::register::info::PropertyUsageFlags;
use std::path::{Path, PathBuf};

const STATUS_OPTION: &str = "godot_cef/runtime";

#[derive(GodotClass)]
#[class(tool, base=EditorExportPlugin, no_init)]
pub struct CefRuntimeExport {
    base: Base<EditorExportPlugin>,
    addon: PathBuf,
    resource_prefix: String,
}

pub fn create(addon: PathBuf) -> Gd<CefRuntimeExport> {
    let resource_prefix = ProjectSettings::singleton()
        .localize_path(&addon.to_string_lossy().replace('\\', "/"))
        .to_string()
        .trim_end_matches('/')
        .to_owned()
        + "/";
    Gd::from_init_fn(|base| CefRuntimeExport {
        base,
        addon,
        resource_prefix,
    })
}

#[godot_api]
impl IEditorExportPlugin for CefRuntimeExport {
    fn customize_resource(
        &mut self,
        _resource: Gd<Resource>,
        _path: GString,
    ) -> Option<Gd<Resource>> {
        None
    }

    fn customize_scene(&mut self, _scene: Gd<Node>, _path: GString) -> Option<Gd<Node>> {
        None
    }

    fn get_customization_configuration_hash(&self) -> u64 {
        0
    }

    fn get_name(&self) -> GString {
        "GodotCefRuntime".into()
    }

    fn supports_platform(&self, platform: Option<Gd<EditorExportPlatform>>) -> bool {
        platform.is_some_and(|platform| {
            matches!(
                platform.get_os_name().to_string().as_str(),
                "Windows" | "Linux" | "macOS"
            )
        })
    }

    fn get_export_options(
        &self,
        _platform: Option<Gd<EditorExportPlatform>>,
    ) -> Array<AnyDictionary> {
        let option = vdict! {
            "name" => STATUS_OPTION,
            "type" => VariantType::STRING.ord(),
            "usage" => (PropertyUsageFlags::EDITOR.ord() | PropertyUsageFlags::READ_ONLY.ord()) as i64,
        };
        let entry = vdict! {
            "option" => &option,
            "default_value" => "Managed by the Godot CEF installer",
        };
        Array::from(&[entry.upcast_any_dictionary()])
    }

    fn get_export_option_warning(
        &self,
        platform: Option<Gd<EditorExportPlatform>>,
        option: GString,
    ) -> GString {
        if option != STATUS_OPTION {
            return GString::new();
        }
        let Some(platform) = platform else {
            return GString::new();
        };
        let os = platform.get_os_name().to_string();
        let architecture = self
            .base()
            .get_option("binary_format/architecture")
            .try_to::<GString>()
            .map(|value| value.to_string())
            .unwrap_or_default();
        let result = target_for_os_arch(&os, &architecture)
            .and_then(|target| check_runtime(&self.addon, target));
        result
            .err()
            .map_or_else(GString::new, |message| message.as_str().into())
    }

    fn export_begin(
        &mut self,
        features: PackedStringArray,
        _is_debug: bool,
        _path: GString,
        _flags: u32,
    ) {
        let features: Vec<String> = features
            .as_slice()
            .iter()
            .map(ToString::to_string)
            .collect();
        let target = match target_for_features(&features) {
            Ok(target) => target,
            Err(message) => {
                self.report_error(&message);
                return;
            }
        };
        let manifest = match Manifest::load(&self.addon) {
            Ok(manifest) => manifest,
            Err(error) => {
                self.report_error(&format!(
                    "Cannot read the Godot CEF runtime manifest: {error}"
                ));
                return;
            }
        };
        let result = gdcef_runtime::prepare_export(&self.addon, &manifest, target)
            .map_err(|error| error.to_string())
            .and_then(|()| crate::host_core_loaded(&self.addon, &manifest));
        if let Err(error) = result {
            self.report_error(&format!(
                "Godot CEF runtime for {target} is not ready: {error}. Install or repair this platform in the Godot CEF installer before exporting."
            ));
            if let Ok(platform) = manifest.platform(target) {
                // Register only real, required paths. This delegates failure to
                // Godot's existing native-file copier without fabricated files
                // or changes to user presets.
                for path in missing_dependencies(&self.addon, platform) {
                    self.base_mut().add_shared_object(
                        &path.to_string_lossy().replace('\\', "/"),
                        &PackedStringArray::new(),
                        "",
                    );
                }
            }
        }
    }

    fn export_file(&mut self, path: GString, _type_: GString, _features: PackedStringArray) {
        if let Some(relative) = path.to_string().strip_prefix(&self.resource_prefix)
            && (relative == "runtime-install.json"
                || relative.ends_with(".gdextension.in")
                || relative.ends_with(".gdextension.template")
                || relative.ends_with(".runtime-installed.json")
                || relative.ends_with(".tar.gz"))
        {
            self.base_mut().skip();
        }
    }
}

impl CefRuntimeExport {
    fn report_error(&self, message: &str) {
        godot_error!("[Godot CEF Export] {message}");
        if let Some(mut platform) = self.base().get_export_platform() {
            platform.add_message(ExportMessageType::ERROR, "Godot CEF", message);
        }
    }
}

fn check_runtime(addon: &Path, target: &str) -> Result<(), String> {
    let manifest = Manifest::load(addon).map_err(|error| error.to_string())?;
    manifest
        .platform(target)
        .map_err(|error| error.to_string())?;
    let status = gdcef_runtime::statuses(addon, &manifest)
        .into_iter()
        .find(|status| status.target == target)
        .ok_or_else(|| format!("Godot CEF does not support {target}"))?;
    if status.installed {
        // Exporting a foreign runtime still imports and serializes resources in
        // this editor. Its browser classes must come from the current package.
        crate::host_core_loaded(addon, &manifest)
    } else {
        Err(format!(
            "Godot CEF runtime for {target}: {}. Open the Godot CEF installer to prepare this platform.",
            status.message
        ))
    }
}

fn target_for_features(features: &[String]) -> Result<&'static str, String> {
    let has = |value: &str| features.iter().any(|feature| feature == value);
    let os = if has("windows") {
        "Windows"
    } else if has("linux") {
        "Linux"
    } else if has("macos") {
        "macOS"
    } else {
        return Err("Godot CEF supports Windows, Linux and macOS desktop exports.".into());
    };
    let architecture = if has("arm64") {
        "arm64"
    } else if has("x86_64") {
        "x86_64"
    } else {
        ""
    };
    target_for_os_arch(os, architecture)
}

fn target_for_os_arch(os: &str, architecture: &str) -> Result<&'static str, String> {
    match (os, architecture) {
        ("macOS", _) => Ok("universal-apple-darwin"),
        ("Windows", "x86_64") => Ok("x86_64-pc-windows-msvc"),
        ("Windows", "arm64") => Ok("aarch64-pc-windows-msvc"),
        ("Linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("Linux", "arm64") => Ok("aarch64-unknown-linux-gnu"),
        _ => Err(format!(
            "Godot CEF does not support the export target {os}/{architecture}."
        )),
    }
}

fn missing_dependencies(addon: &Path, platform: &Platform) -> Vec<PathBuf> {
    let directory = addon.join(&platform.directory);
    let mut missing = Vec::new();
    let library = addon.join(&platform.library);
    if !library.is_file() {
        missing.push(library);
    }
    missing.extend(
        platform
            .files
            .iter()
            .map(|file| directory.join(file))
            .filter(|file| !file.is_file()),
    );
    missing.extend(
        platform
            .dirs
            .iter()
            .map(|dir| directory.join(dir))
            .filter(|dir| !dir.is_dir()),
    );
    missing
}

#[cfg(test)]
#[allow(clippy::panic_in_result_fn)] // Assertions intentionally fail the test harness.
mod tests {
    use super::*;

    #[test]
    fn export_target_uses_export_features_instead_of_host_architecture() -> Result<(), String> {
        assert_eq!(
            target_for_features(&["windows".into(), "arm64".into()])?,
            "aarch64-pc-windows-msvc"
        );
        assert_eq!(
            target_for_features(&["linux".into(), "x86_64".into()])?,
            "x86_64-unknown-linux-gnu"
        );
        assert_eq!(
            target_for_features(&["macos".into(), "universal".into()])?,
            "universal-apple-darwin"
        );
        assert!(target_for_features(&["windows".into(), "x86_32".into()]).is_err());
        assert!(target_for_features(&["android".into(), "arm64".into()]).is_err());
        Ok(())
    }
}
