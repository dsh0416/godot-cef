//! Custom error types for the CEF Godot extension.
//!
//! This module provides a centralized error type for handling failures
//! in CEF initialization, browser creation, and texture operations.

use std::fmt;
use std::io;

/// Main error type for CEF operations.
#[derive(Debug)]
pub enum CefError {
    /// CEF framework loading failed. only occurs on macOS.
    #[cfg(target_os = "macos")]
    FrameworkLoadFailed(String),
    // allow dead code on other platforms
    #[allow(dead_code)]
    #[cfg(not(target_os = "macos"))]
    FrameworkLoadFailed(String),
    /// CEF initialization failed.
    InitializationFailed(String),
    /// Browser creation failed.
    BrowserCreationFailed(String),
    /// Texture import or copy operation failed.
    TextureOperationFailed(String),
    /// Path resolution failed.
    PathError(io::Error),
    /// A required resource was not found.
    ResourceNotFound(String),
    /// GPU device access failed.
    GpuDeviceError(String),
}

impl fmt::Display for CefError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CefError::FrameworkLoadFailed(msg) => {
                write!(f, "Failed to load CEF framework: {}", msg)
            }
            CefError::InitializationFailed(msg) => {
                write!(f, "Failed to initialize CEF: {}", msg)
            }
            CefError::BrowserCreationFailed(msg) => {
                write!(f, "Failed to create browser: {}", msg)
            }
            CefError::TextureOperationFailed(msg) => {
                write!(f, "Texture operation failed: {}", msg)
            }
            CefError::PathError(err) => {
                write!(f, "Path error: {}", err)
            }
            CefError::ResourceNotFound(resource) => {
                write!(f, "Resource not found: {}", resource)
            }
            CefError::GpuDeviceError(msg) => {
                write!(f, "GPU device error: {}", msg)
            }
        }
    }
}

impl std::error::Error for CefError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CefError::PathError(err) => Some(err),
            CefError::FrameworkLoadFailed(_)
            | CefError::InitializationFailed(_)
            | CefError::BrowserCreationFailed(_)
            | CefError::TextureOperationFailed(_)
            | CefError::ResourceNotFound(_)
            | CefError::GpuDeviceError(_) => None,
        }
    }
}

impl From<io::Error> for CefError {
    fn from(err: io::Error) -> Self {
        CefError::PathError(err)
    }
}

/// Result type alias for CEF operations.
pub type CefResult<T> = Result<T, CefError>;
