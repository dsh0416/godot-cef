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

// Request discrete GPU on Windows laptops with hybrid graphics
#[cfg(windows)]
#[no_mangle]
#[used]
pub static NvOptimusEnablement: u32 = 0x00000001;

#[cfg(windows)]
#[no_mangle]
#[used]
pub static AmdPowerXpressRequestHighPerformance: u32 = 0x00000001;

struct GodotCef;

#[gdextension]
unsafe impl ExtensionLibrary for GodotCef {}

// Re-export CefTexture for convenience
pub use cef_texture::CefTexture;
