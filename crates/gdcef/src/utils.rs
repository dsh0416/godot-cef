use crate::error::{CefError, CefResult};
use godot::classes::Engine;
use godot::classes::Os;
use godot::{classes::DisplayServer, obj::Singleton};
use process_path::get_dylib_path;
use std::path::PathBuf;
use std::sync::OnceLock;

#[cfg(target_os = "linux")]
static LINUX_DESKTOP_SCALE_CANDIDATE: OnceLock<Option<f32>> = OnceLock::new();

/// Returns the display scale factor for the primary screen.
///
/// This value can be used to scale UI elements from logical pixels to
/// physical pixels in order to appear consistent across different DPI
/// and high-DPI displays. A value of `1.0` means "no scaling".
pub fn get_display_scale_factor() -> f32 {
    let display_server = DisplayServer::singleton();
    let screen_scale = display_server.screen_get_scale();

    // NOTE: `display_server.screen_get_scale` is implemented on Android, iOS,
    // Web, macOS, and Linux (Wayland). On Windows, this method always returns
    // 1.0, so we derive the scale from the screen DPI instead.
    #[cfg(target_os = "windows")]
    {
        let dpi = display_server.screen_get_dpi();
        if dpi > 0 {
            (dpi as f32 / 96.0).max(1.0)
        } else {
            1.0
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        #[cfg(target_os = "linux")]
        {
            if screen_scale <= 1.0
                && env_or_empty("XDG_SESSION_TYPE").eq_ignore_ascii_case("wayland")
                && let Some(candidate) = linux_desktop_scale_candidate()
                && candidate > 1.0
            {
                candidate
            } else {
                screen_scale
            }
        }

        #[cfg(not(target_os = "linux"))]
        {
            screen_scale
        }
    }
}

#[cfg(target_os = "linux")]
fn linux_desktop_scale_candidate() -> Option<f32> {
    *LINUX_DESKTOP_SCALE_CANDIDATE.get_or_init(|| {
        let scales = gnome_monitors_xml_scales();
        scales
            .into_iter()
            .filter(|scale| scale.is_finite() && *scale > 1.0)
            .max_by(|a, b| a.total_cmp(b))
    })
}

#[cfg(target_os = "linux")]
fn gnome_monitors_xml_scales() -> Vec<f32> {
    use quick_xml::Reader;
    use quick_xml::events::Event;

    let Some(home) = std::env::var_os("HOME") else {
        return Vec::new();
    };
    let path = PathBuf::from(home).join(".config/monitors.xml");
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut reader = Reader::from_str(&contents);
    reader.config_mut().trim_text(true);

    let mut scales = Vec::new();
    let mut in_scale = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) if element.name().as_ref() == b"scale" => {
                in_scale = true;
            }
            Ok(Event::End(element)) if element.name().as_ref() == b"scale" => {
                in_scale = false;
            }
            Ok(Event::Text(text)) if in_scale => {
                if let Ok(value) = text.decode()
                    && let Ok(scale) = value.parse::<f32>()
                {
                    scales.push(scale);
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => return Vec::new(),
            _ => {}
        }
    }
    scales
}

#[cfg(target_os = "linux")]
fn env_or_empty(name: &str) -> String {
    std::env::var(name).unwrap_or_default()
}

fn get_dylib_path_checked() -> CefResult<PathBuf> {
    get_dylib_path().ok_or_else(|| CefError::ResourceNotFound("dylib path".to_string()))
}

fn get_dylib_dir() -> CefResult<PathBuf> {
    let dylib_path = get_dylib_path_checked()?;
    dylib_path
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| CefError::ResourceNotFound("dylib directory".to_string()))
}

#[cfg(target_os = "macos")]
pub fn get_framework_path() -> CefResult<PathBuf> {
    let dylib_dir = get_dylib_dir()?;

    let framework_name = match std::env::consts::ARCH {
        "aarch64" => "Chromium Embedded Framework (ARM64).framework",
        "x86_64" => "Chromium Embedded Framework (X86_64).framework",
        arch => {
            return Err(CefError::ResourceNotFound(format!(
                "Unsupported architecture: {}",
                arch
            )));
        }
    };

    // current dylib path:
    //   project/addons/godot_cef/bin/universal-apple-darwin/Godot CEF.framework/libgdcef.dylib
    // framework is at:
    //   project/addons/godot_cef/bin/universal-apple-darwin/Godot CEF.app/Contents/Frameworks/Chromium Embedded Framework (ARM64|X86_64).framework
    dylib_dir
        .join("..")
        .join("Godot CEF.app/Contents/Frameworks")
        .join(framework_name)
        .canonicalize()
        .map_err(CefError::from)
}

#[cfg(target_os = "macos")]
pub fn get_subprocess_path() -> CefResult<PathBuf> {
    let dylib_dir = get_dylib_dir()?;

    // current dylib path:
    //   project/addons/godot_cef/bin/universal-apple-darwin/Godot CEF.framework/libgdcef.dylib
    // subprocess is at:
    //   project/addons/godot_cef/bin/universal-apple-darwin/Godot CEF.app/Contents/Frameworks/Godot CEF Helper.app/Contents/MacOS/Godot CEF Helper
    dylib_dir
        .join("..")
        .join("Godot CEF.app/Contents/Frameworks")
        .join("Godot CEF Helper.app/Contents/MacOS")
        .join("Godot CEF Helper")
        .canonicalize()
        .map_err(CefError::from)
}

#[cfg(target_os = "windows")]
pub fn get_subprocess_path() -> CefResult<PathBuf> {
    let dylib_dir = get_dylib_dir()?;

    // current dylib path:
    //   project/addons/godot_cef/bin/x86_64-pc-windows-msvc/gdcef.dll
    // subprocess is at:
    //   project/addons/godot_cef/bin/x86_64-pc-windows-msvc/gdcef_helper.exe
    dylib_dir
        .join("gdcef_helper.exe")
        .canonicalize()
        .map_err(CefError::from)
}

#[cfg(target_os = "linux")]
pub fn get_subprocess_path() -> CefResult<PathBuf> {
    let dylib_dir = get_dylib_dir()?;

    // current dylib path:
    //   project/addons/godot_cef/bin/x86_64-unknown-linux-gnu/libgdcef.so
    // subprocess is at:
    //   project/addons/godot_cef/bin/x86_64-unknown-linux-gnu/gdcef_helper
    dylib_dir
        .join("gdcef_helper")
        .canonicalize()
        .map_err(CefError::from)
}

#[cfg(unix)]
pub fn ensure_executable_permissions() -> CefResult<()> {
    use std::os::unix::fs::PermissionsExt;

    let paths_to_make_executable = get_executable_paths()?;

    for path in paths_to_make_executable {
        if !path.exists() {
            godot::global::godot_warn!(
                "[CefInit] Executable not found, skipping: {}",
                path.display()
            );
            continue;
        }

        let metadata = std::fs::metadata(&path).map_err(|e| {
            CefError::ResourceNotFound(format!(
                "Failed to get metadata for {}: {}",
                path.display(),
                e
            ))
        })?;

        let mut permissions = metadata.permissions();
        let current_mode = permissions.mode();
        let new_mode = current_mode | ((current_mode & 0o444) >> 2);

        if current_mode != new_mode {
            permissions.set_mode(new_mode);
            std::fs::set_permissions(&path, permissions).map_err(|e| {
                CefError::InitializationFailed(format!(
                    "Failed to set executable permissions for {}: {}",
                    path.display(),
                    e
                ))
            })?;
            godot::global::godot_print!(
                "[CefInit] Set executable permissions for: {}",
                path.display()
            );
        }
    }

    Ok(())
}

#[cfg(not(unix))]
pub fn ensure_executable_permissions() -> CefResult<()> {
    Ok(())
}

#[cfg(unix)]
fn get_executable_paths() -> CefResult<Vec<PathBuf>> {
    let mut paths = Vec::new();

    let subprocess_path = get_subprocess_path()?;
    paths.push(subprocess_path.clone());

    #[cfg(target_os = "linux")]
    {
        let dylib_path = get_dylib_path_checked()?;
        let chrome_sandbox = dylib_path.join("../chrome-sandbox");
        if let Ok(canonical) = chrome_sandbox.canonicalize() {
            paths.push(canonical);
        }
        let gdcef_helper = dylib_path.join("../gdcef_helper");
        if let Ok(canonical) = gdcef_helper.canonicalize() {
            paths.push(canonical);
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(frameworks_dir) = subprocess_path
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
        {
            let helper_variants = [
                "Godot CEF Helper (GPU)",
                "Godot CEF Helper (Renderer)",
                "Godot CEF Helper (Plugin)",
                "Godot CEF Helper (Alerts)",
            ];

            for variant in &helper_variants {
                let variant_path = frameworks_dir
                    .join(format!("{}.app", variant))
                    .join("Contents/MacOS")
                    .join(variant);

                if variant_path.exists() {
                    paths.push(variant_path);
                }
            }
        }
    }

    Ok(paths)
}

/// Determines if IPC inspector should be enabled.
///
/// IPC inspector is only enabled when:
/// - Godot is compiled in debug mode (Os.is_debug_build() returns true), OR
/// - The game is running from the Godot editor (Engine.is_editor_hint() returns true)
///
/// This is a security measure to prevent remote debugging in production builds.
pub(crate) fn should_enable_ipc_inspector() -> bool {
    let os = Os::singleton();
    let engine = Engine::singleton();

    let is_debug_build = os.is_debug_build();
    let is_editor_hint = engine.is_editor_hint();

    is_debug_build || is_editor_hint
}
