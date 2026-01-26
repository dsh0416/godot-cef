mod accelerated_osr;
mod browser;
mod cef_init;
mod cef_texture;
mod cursor;
mod drag;
mod error;
mod godot_protocol;
mod input;
mod queue_processing;
mod render;
mod settings;
mod utils;
mod vulkan_hook;
mod webrender;

use godot::init::*;

struct GodotCef;

#[gdextension]
unsafe impl ExtensionLibrary for GodotCef {
    fn on_stage_init(level: InitStage) {
        match level {
            InitStage::Core => {
                // Install Vulkan hook at the Core stage, BEFORE RenderingServer is created.
                // This allows us to inject platform-specific external memory extensions
                // (e.g., VK_KHR_external_memory_win32 and related Win32 external memory extensions) into Godot's Vulkan device.
                vulkan_hook::install_vulkan_hook();

                if let Err(e) = utils::ensure_executable_permissions() {
                    godot::global::godot_warn!(
                        "[GodotCef] Failed to set executable permissions: {}",
                        e
                    );
                }
            }
            InitStage::Scene => {
                settings::register_project_settings();
            }
            _ => {}
        }
    }
}

// Re-export CefTexture for convenience
pub use cef_texture::CefTexture;
