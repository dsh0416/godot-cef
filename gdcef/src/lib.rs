mod accelerated_osr;
mod browser;
mod cef_init;
mod cef_texture;
mod cursor;
mod error;
mod input;
mod queue_processing;
mod render;
mod res_protocol;
mod utils;
mod webrender;

use godot::init::*;

struct GodotCef;

#[gdextension]
unsafe impl ExtensionLibrary for GodotCef {}

// Re-export CefTexture for convenience
pub use cef_texture::CefTexture;
