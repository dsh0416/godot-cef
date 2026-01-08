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

    // current dylib path:
    //   project/addons/godot_cef/bin/universal-apple-darwin/Godot CEF.framework/libgdcef.dylib
    // framework is at:
    //   project/addons/godot_cef/bin/universal-apple-darwin/Godot CEF.app/Contents/Frameworks/Chromium Embedded Framework.framework
    dylib_path
        .join("../..")
        .join("Godot CEF.app/Contents/Frameworks")
        .join("Chromium Embedded Framework.framework")
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
