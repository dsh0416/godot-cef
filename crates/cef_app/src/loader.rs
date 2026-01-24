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
/// # Panics
/// Panics if the framework cannot be loaded.
///
/// # Safety
/// This function calls the CEF C API directly to load the library. The path must
/// point to a valid CEF framework.
#[cfg(target_os = "macos")]
pub fn load_cef_framework_from_path(framework_path: &Path) {
    use cef::sys::cef_load_library;

    let path = framework_path
        .join("Chromium Embedded Framework")
        .canonicalize()
        .expect("Failed to canonicalize CEF framework path");

    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .expect("Failed to convert library path to CString");

    // SAFETY: We're calling the CEF C API with a valid path. The path has been
    // validated above by canonicalize(). The cef_load_library function is
    // documented to safely load the framework or return an error code.
    let result = unsafe {
        let arg_path = Some(&*path.as_ptr().cast());
        let arg_path = arg_path.map(std::ptr::from_ref).unwrap_or(std::ptr::null());
        cef_load_library(arg_path) == 1
    };

    assert!(result, "Failed to load macOS CEF framework");
}

/// No-op on non-macOS platforms.
#[cfg(not(target_os = "macos"))]
pub fn load_cef_framework_from_path(_framework_path: &Path) {
    // CEF is linked directly on Windows and Linux
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
pub fn load_sandbox_from_path(framework_path: &Path, args: &cef::MainArgs) {
    use libloading::Library;

    let path = framework_path
        .join("Libraries/libcef_sandbox.dylib")
        .canonicalize()
        .expect("Failed to canonicalize sandbox library path");

    // SAFETY: We're loading a known CEF library and calling its documented
    // initialization function. The library path has been validated.
    unsafe {
        let lib = Library::new(path).expect("Failed to load CEF sandbox library");
        let func =
            lib.get::<unsafe extern "C" fn(
                argc: std::os::raw::c_int,
                argv: *mut *mut ::std::os::raw::c_char,
            )>(b"cef_sandbox_initialize\0")
                .expect("Failed to find cef_sandbox_initialize function");
        func(args.argc, args.argv);
    }
}

/// No-op on non-macOS platforms.
#[cfg(not(target_os = "macos"))]
pub fn load_sandbox_from_path(_framework_path: &Path, _args: &cef::MainArgs) {
    // Sandbox is handled differently on Windows and Linux
}
