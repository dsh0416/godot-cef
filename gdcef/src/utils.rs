use godot::{classes::DisplayServer, obj::Singleton};
use process_path::get_dylib_path;
use std::{io::Error, path::PathBuf};

/// Returns the display scale factor for the primary screen.
///
/// This value can be used to scale UI elements from logical pixels to
/// physical pixels in order to appear consistent across different DPI
/// and high-DPI displays. A value of `1.0` means "no scaling".
///
/// # Parameters
///
/// This function does not take any parameters.
///
/// # Returns
///
/// A `f32` scale factor for the current display. The value is always
/// greater than or equal to `1.0`.
///
/// # Platform-specific behavior
///
/// - **Windows**: The scale factor is derived from the screen DPI, using
///   96 DPI as the baseline. If the DPI cannot be determined or is
///   reported as `0`, this function falls back to `1.0`.
/// - **Other platforms** (Android, iOS, Web, macOS, Linux/Wayland, etc.):
///   The value is provided by `DisplayServer::screen_get_scale()`. On
///   platforms where `screen_get_scale()` is not implemented, it always
///   returns `1.0`.
pub fn get_display_scale_factor() -> f32 {
    let display_server = DisplayServer::singleton();

    // NOTE: `display_server.screen_get_scale` is implemented on Android, iOS, Web, macOS, and Linux (Wayland). On Windows, this method always returns 1.0, so we derive the scale from the screen DPI instead.
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

#[cfg(target_os = "macos")]
pub fn get_framework_path() -> Result<PathBuf, Error> {
    let dylib_path = get_dylib_path();

    // current dylib path is project/addons/godot_cef/bin/universal-apple-darwin/Godot CEF.framework/libgdcef.dylib
    // framework is at project/addons/godot_cef/bin/universal-apple-darwin/Godot CEF.app/Contents/Frameworks/Chromium Embedded Framework.framework
    dylib_path
        .unwrap()
        .join("../..")
        .join("Godot CEF.app/Contents/Frameworks")
        .join("Chromium Embedded Framework.framework")
        .canonicalize()
}

#[cfg(target_os = "macos")]
pub fn get_subprocess_path() -> Result<PathBuf, Error> {
    let dylib_path = get_dylib_path();

    // current dylib path is project/addons/godot_cef/bin/universal-apple-darwin/Godot CEF.framework/libgdcef.dylib
    // subprocess is at project/addons/godot_cef/bin/universal-apple-darwin/Godot CEF.app/Contents/MacOS/Godot CEF
    dylib_path
        .unwrap()
        .join("../..")
        .join("Godot CEF.app/Contents/Frameworks")
        .join("Godot CEF Helper.app/Contents/MacOS")
        .join("Godot CEF Helper")
        .canonicalize()
}

#[cfg(target_os = "windows")]
pub fn get_subprocess_path() -> Result<PathBuf, Error> {
    let dylib_path = get_dylib_path();

    // current dylib path is project/addons/godot_cef/bin/x86_64-pc-windows-msvc/gdcef.dll
    // subprocess is at project/addons/godot_cef/bin/x86_64-pc-windows-msvc/gdcef_helper.exe
    dylib_path
        .unwrap()
        .join("../gdcef_helper.exe")
        .canonicalize()
}

#[cfg(target_os = "linux")]
pub fn get_subprocess_path() -> Result<PathBuf, Error> {
    let dylib_path = get_dylib_path();

    // current dylib path is project/addons/godot_cef/bin/x86_64-unknown-linux-gnu/libgdcef.so
    // subprocess is at project/addons/godot_cef/bin/x86_64-unknown-linux-gnu/gdcef_helper
    dylib_path.unwrap().join("../gdcef_helper").canonicalize()
}
