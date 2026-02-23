//! CEF framework loading utilities.
//!
//! This module provides shared functionality for loading the CEF framework
//! and sandbox on different platforms.

use std::path::Path;

/// Loads the CEF framework library from the given path (macOS-specific).
///
/// # Arguments
/// * `framework_path` - Path to the `Chromium Embedded Framework.framework` directory.
///
/// # Safety
/// This function calls the CEF C API directly to load the library. The path must
/// point to a valid CEF framework.
#[cfg(target_os = "macos")]
pub fn load_cef_framework_from_path(framework_path: &Path) -> Result<(), String> {
    use cef::sys::cef_load_library;

    let path = framework_path
        .join("Chromium Embedded Framework")
        .canonicalize()
        .map_err(|e| format!("Failed to canonicalize CEF framework path: {e}"))?;

    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|e| format!("Failed to convert library path to CString: {e}"))?;

    // SAFETY: We're calling the CEF C API with a valid path. The path has been
    // validated above by canonicalize(). The cef_load_library function is
    // documented to safely load the framework or return an error code.
    let result = unsafe {
        let arg_path = Some(&*path.as_ptr().cast());
        let arg_path = arg_path.map(std::ptr::from_ref).unwrap_or(std::ptr::null());
        cef_load_library(arg_path) == 1
    };

    if !result {
        return Err("Failed to load macOS CEF framework".to_string());
    }
    Ok(())
}

/// No-op on non-macOS platforms.
#[cfg(not(target_os = "macos"))]
pub fn load_cef_framework_from_path(_framework_path: &Path) -> Result<(), String> {
    // CEF is linked directly on Windows and Linux
    Ok(())
}

/// Loads the CEF sandbox from the given framework path (macOS-specific).
///
/// # Arguments
/// * `framework_path` - Path to the `Chromium Embedded Framework.framework` directory.
/// * `args` - The main args for the CEF process.
///
/// # Safety
/// This function dynamically loads and calls the CEF sandbox initialization function.
/// The framework_path must point to a valid CEF framework containing the sandbox library.
#[cfg(target_os = "macos")]
pub fn load_sandbox_from_path(framework_path: &Path, args: &cef::MainArgs) -> Result<(), String> {
    use libloading::Library;

    let path = framework_path
        .join("Libraries/libcef_sandbox.dylib")
        .canonicalize()
        .map_err(|e| format!("Failed to canonicalize sandbox library path: {e}"))?;

    // SAFETY: We're loading a known CEF library and calling its documented
    // initialization function. The library path has been validated.
    unsafe {
        let lib =
            Library::new(path).map_err(|e| format!("Failed to load CEF sandbox library: {e}"))?;
        let func =
            lib.get::<unsafe extern "C" fn(
                argc: std::os::raw::c_int,
                argv: *mut *mut ::std::os::raw::c_char,
            )>(b"cef_sandbox_initialize\0")
                .map_err(|e| format!("Failed to find cef_sandbox_initialize function: {e}"))?;
        func(args.argc, args.argv);
    }
    Ok(())
}

/// No-op on non-macOS platforms.
#[cfg(not(target_os = "macos"))]
pub fn load_sandbox_from_path(_framework_path: &Path, _args: &cef::MainArgs) -> Result<(), String> {
    // Sandbox is handled differently on Windows and Linux
    Ok(())
}
