#[cfg(not(target_os = "android"))]
mod desktop;
mod mobile_webview;

#[cfg(not(target_os = "android"))]
pub(crate) use desktop::{
    accelerated_osr, browser, cef_init, cef_ipc_inspector, cef_texture, cef_texture2d, compat,
    cookie, cursor, drag, error, godot_protocol, input, ipc_data, render, settings, utils,
    webrender, webrender_ipc,
};

use godot::init::*;

struct GodotCef;

#[gdextension]
unsafe impl ExtensionLibrary for GodotCef {
    fn on_stage_init(level: InitStage) {
        #[cfg(not(target_os = "android"))]
        desktop::on_stage_init(level);

        #[cfg(target_os = "android")]
        let _ = level;
    }
}

// Re-export CefTexture for convenience
#[cfg(not(target_os = "android"))]
pub use cef_ipc_inspector::CefIpcInspector;
#[cfg(not(target_os = "android"))]
pub use cef_texture::CefTexture;
#[cfg(not(target_os = "android"))]
pub use cef_texture2d::CefTexture2D;
pub use mobile_webview::AndroidWebViewTexture;
