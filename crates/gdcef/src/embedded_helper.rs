//! Materialize the matching helper before starting CEF.
//!
//! Distribution builds embed the complete helper archive in the core library.
//! The installer can also extract these same bytes without loading the library,
//! which is necessary when preparing an export for a different architecture.

use crate::error::{CefError, CefResult};
#[cfg(not(target_os = "macos"))]
use std::path::{Path, PathBuf};

include!(concat!(env!("OUT_DIR"), "/embedded_helper.rs"));

pub fn ensure_available() -> CefResult<()> {
    #[cfg(target_os = "macos")]
    {
        // The installer expands the apps before signing the complete framework.
        // Signing can change their Mach-O bytes. Exported games must not compare
        // those bytes to the pre-signing payload or rewrite their signed bundle.
        // Retain a live reference through LTO/dead stripping: the installer
        // extracts this payload from the dylib without loading foreign code.
        std::hint::black_box((&HELPER_ARCHIVE, HELPER_SHA256));
        crate::utils::get_subprocess_path().map(|_| ()).map_err(|error| {
            CefError::InitializationFailed(format!(
                "The signed CEF helper bundle is missing ({error}). Repair the runtime in the Godot CEF installer and export the project again."
            ))
        })
    }
    #[cfg(not(target_os = "macos"))]
    ensure_available_native()
}

#[cfg(not(target_os = "macos"))]
fn ensure_available_native() -> CefResult<()> {
    if HELPER_ARCHIVE.is_empty() {
        // A plain Cargo build deliberately does not require the packaging step.
        // It must still have a usable sidecar helper; do not silently continue
        // with a missing payload and defer the failure to the CEF subprocess.
        return crate::utils::get_subprocess_path().map(|_| ()).map_err(|error| {
            CefError::InitializationFailed(format!(
                "No embedded helper or development helper is available ({error}). Build the addon with `mise run bundle`."
            ))
        });
    }

    let library = process_path::get_dylib_path()
        .ok_or_else(|| CefError::ResourceNotFound("dylib path".to_string()))?;
    let destination = payload_directory(&library)?;
    gdcef_runtime::ensure_helper_payload(&HELPER_ARCHIVE, HELPER_SHA256, &destination).map_err(
        |error| {
            CefError::InitializationFailed(format!(
                "Failed to prepare the embedded helper: {error}"
            ))
        },
    )?;
    crate::utils::get_subprocess_path().map(|_| ())
}

#[cfg(not(target_os = "macos"))]
fn payload_directory(library: &Path) -> CefResult<PathBuf> {
    let directory = library
        .parent()
        .ok_or_else(|| CefError::ResourceNotFound("dylib directory".to_string()))?;
    Ok(directory.to_path_buf())
}
