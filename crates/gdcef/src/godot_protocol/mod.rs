//! Custom Godot scheme handlers for CEF.
//!
//! This module implements custom scheme handlers that allow CEF to load
//! resources from Godot's filesystem using `res://` and `user://` protocols.
//! This enables exported Godot projects to serve local web content (HTML, CSS,
//! JS, images, etc.) directly to the embedded browser without requiring an
//! external web server.
//!
//! - `res://` - Access resources from Godot's packed resource system
//! - `user://` - Access files from Godot's user data directory

mod handler;
mod mime;
mod multipart;
mod range;

pub use handler::{
    register_res_scheme_handler_on_context, register_user_scheme_handler_on_context,
};

/// Represents the Godot filesystem scheme type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GodotScheme {
    /// Access resources from Godot's packed resource system (`res://`)
    Res,
    /// Access files from Godot's user data directory (`user://`)
    User,
}

impl GodotScheme {
    pub(crate) fn prefix(&self) -> &'static str {
        match self {
            GodotScheme::Res => "res://",
            GodotScheme::User => "user://",
        }
    }

    pub(crate) fn short_prefix(&self) -> &'static str {
        match self {
            GodotScheme::Res => "res:",
            GodotScheme::User => "user:",
        }
    }

    pub(crate) fn name(&self) -> &'static str {
        match self {
            GodotScheme::Res => "res",
            GodotScheme::User => "user",
        }
    }
}
