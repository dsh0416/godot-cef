#[path = "accelerated_osr/mod.rs"]
pub(crate) mod accelerated_osr;
#[path = "browser.rs"]
pub(crate) mod browser;
#[path = "cef_init.rs"]
pub(crate) mod cef_init;
#[path = "cef_ipc_inspector.rs"]
pub(crate) mod cef_ipc_inspector;
#[path = "cef_texture/mod.rs"]
pub(crate) mod cef_texture;
#[path = "cef_texture2d/mod.rs"]
pub(crate) mod cef_texture2d;
#[path = "compat.rs"]
pub(crate) mod compat;
#[path = "cookie.rs"]
pub(crate) mod cookie;
#[path = "cursor.rs"]
pub(crate) mod cursor;
#[path = "drag.rs"]
pub(crate) mod drag;
#[path = "error.rs"]
pub(crate) mod error;
#[path = "godot_protocol/mod.rs"]
pub(crate) mod godot_protocol;
#[path = "input/mod.rs"]
pub(crate) mod input;
#[path = "ipc_data.rs"]
pub(crate) mod ipc_data;
#[path = "render.rs"]
pub(crate) mod render;
#[path = "settings.rs"]
pub(crate) mod settings;
#[path = "utils.rs"]
pub(crate) mod utils;
#[path = "vulkan_hook/mod.rs"]
pub(crate) mod vulkan_hook;
#[path = "webrender.rs"]
pub(crate) mod webrender;
#[path = "webrender_ipc.rs"]
pub(crate) mod webrender_ipc;

use godot::init::InitStage;

pub(crate) fn on_stage_init(level: InitStage) {
    match level {
        InitStage::Core => {
            // Install Vulkan hook before RenderingServer is created so Godot's
            // Vulkan device can request platform external-memory extensions.
            vulkan_hook::install_vulkan_hook();

            if let Err(error) = utils::ensure_executable_permissions() {
                godot::global::godot_warn!(
                    "[GodotCef] Failed to set executable permissions: {}",
                    error
                );
            }
        }
        InitStage::Scene => {
            settings::register_project_settings();
        }
        _ => {}
    }
}
