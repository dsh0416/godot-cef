use super::RenderBackend;
use cef::AcceleratedPaintInfo;
use godot::global::godot_warn;
use godot::prelude::*;

pub struct GodotTextureImporter;

impl GodotTextureImporter {
    pub fn new() -> Option<Self> {
        let render_backend = RenderBackend::detect();

        if !render_backend.supports_accelerated_osr() {
            godot_warn!(
                "[AcceleratedOSR/Linux] Render backend {:?} does not support accelerated OSR",
                render_backend
            );
            return None;
        }

        // TODO: Initialize Vulkan with external memory extensions
        godot_warn!("[AcceleratedOSR/Linux] Vulkan texture import not yet implemented");
        None
    }

    pub fn import_and_copy(
        &mut self,
        _info: &AcceleratedPaintInfo,
        _dst_rd_rid: Rid,
    ) -> Result<u64, String> {
        // TODO: Implement Vulkan texture import and copy
        Err("Accelerated OSR not yet implemented on Linux".to_string())
    }

    pub fn is_copy_complete(&self, _copy_id: u64) -> bool {
        true
    }

    pub fn wait_for_all_copies(&self) {}
}

pub fn is_supported() -> bool {
    false
}

unsafe impl Send for GodotTextureImporter {}
unsafe impl Sync for GodotTextureImporter {}
