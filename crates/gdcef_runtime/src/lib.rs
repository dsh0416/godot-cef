//! Shared runtime installation primitives. The browser library uses only the
//! helper extractor; networking is enabled exclusively by the installer feature.

mod helper;
#[cfg(feature = "installer")]
mod installer;
mod manifest;

pub use helper::{ensure_helper_payload, sha256};
#[cfg(feature = "installer")]
pub use installer::{InstallOptions, InstallSource, Progress, install};
pub use manifest::{
    EmbeddedArchive, Manifest, Platform, TargetStatus, activate_extension,
    defer_extension_activation, finalize_extension_activation, host_target, platform_directory,
    preflight_extension_activation, prepare_export, statuses,
};

use std::fmt;

#[derive(Debug)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self(error.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self(error.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn fail(message: impl Into<String>) -> Error {
    Error(message.into())
}
