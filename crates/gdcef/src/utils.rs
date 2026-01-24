use crate::error::{CefError, CefResult};
use godot::{classes::DisplayServer, obj::Singleton};
use process_path::get_dylib_path;
use std::path::PathBuf;

/// Returns the display scale factor for the primary screen.
///
/// This value can be used to scale UI elements from logical pixels to
/// physical pixels in order to appear consistent across different DPI
/// and high-DPI displays. A value of `1.0` means "no scaling".
pub fn get_display_scale_factor() -> f32 {
    let display_server = DisplayServer::singleton();

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
        display_server.screen_get_scale()
    }
}

fn get_dylib_path_checked() -> CefResult<PathBuf> {
    get_dylib_path().ok_or_else(|| CefError::ResourceNotFound("dylib path".to_string()))
}

#[cfg(target_os = "macos")]
pub fn get_framework_path() -> CefResult<PathBuf> {
    let dylib_path = get_dylib_path_checked()?;

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
    dylib_path
        .join("../..")
        .join("Godot CEF.app/Contents/Frameworks")
        .join(framework_name)
        .canonicalize()
        .map_err(CefError::from)
}

#[cfg(target_os = "macos")]
pub fn get_subprocess_path() -> CefResult<PathBuf> {
    let dylib_path = get_dylib_path_checked()?;

    // current dylib path:
    //   project/addons/godot_cef/bin/universal-apple-darwin/Godot CEF.framework/libgdcef.dylib
    // subprocess is at:
    //   project/addons/godot_cef/bin/universal-apple-darwin/Godot CEF.app/Contents/Frameworks/Godot CEF Helper.app/Contents/MacOS/Godot CEF Helper
    dylib_path
        .join("../..")
        .join("Godot CEF.app/Contents/Frameworks")
        .join("Godot CEF Helper.app/Contents/MacOS")
        .join("Godot CEF Helper")
        .canonicalize()
        .map_err(CefError::from)
}

#[cfg(target_os = "windows")]
pub fn get_subprocess_path() -> CefResult<PathBuf> {
    let dylib_path = get_dylib_path_checked()?;

    // current dylib path:
    //   project/addons/godot_cef/bin/x86_64-pc-windows-msvc/gdcef.dll
    // subprocess is at:
    //   project/addons/godot_cef/bin/x86_64-pc-windows-msvc/gdcef_helper.exe
    dylib_path
        .join("../gdcef_helper.exe")
        .canonicalize()
        .map_err(CefError::from)
}

#[cfg(target_os = "linux")]
pub fn get_subprocess_path() -> CefResult<PathBuf> {
    let dylib_path = get_dylib_path_checked()?;

    // current dylib path:
    //   project/addons/godot_cef/bin/x86_64-unknown-linux-gnu/libgdcef.so
    // subprocess is at:
    //   project/addons/godot_cef/bin/x86_64-unknown-linux-gnu/gdcef_helper
    dylib_path
        .join("../gdcef_helper")
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

/// Attempts to acquire a mutex lock, logging a warning on failure.
macro_rules! try_lock {
    ($mutex:expr, $context:literal) => {
        match $mutex.lock() {
            Ok(guard) => Some(guard),
            Err(e) => {
                godot::global::godot_warn!(
                    "[CefTexture] Failed to acquire lock for {}: {}",
                    $context,
                    e
                );
                None
            }
        }
    };
}

pub(crate) use try_lock;
