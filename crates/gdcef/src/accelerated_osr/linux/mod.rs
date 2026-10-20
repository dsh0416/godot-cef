//! Linux-specific accelerated OSR implementation.
//!
//! On Linux, we use Vulkan with DMA-BUF external memory extensions to import
//! shared textures from CEF's compositor process.

mod vulkan;

use super::{NativeCaptureTarget, RenderBackend};
use cef::AcceleratedPaintInfo;
use godot::global::godot_print;

pub fn get_godot_gpu_device_ids() -> Option<(u32, u32)> {
    vulkan::get_godot_gpu_device_ids()
}

pub struct GodotTextureImporter {
    vulkan_importer: vulkan::VulkanTextureImporter,
}

impl GodotTextureImporter {
    pub fn new() -> Result<Self, String> {
        let render_backend = RenderBackend::detect();

        match render_backend {
            RenderBackend::Vulkan => {
                let vulkan_importer = vulkan::VulkanTextureImporter::new()?;
                godot_print!("[AcceleratedOSR/Linux] Using Vulkan backend with DMA-BUF");
                Ok(Self { vulkan_importer })
            }
            _ => Err(format!(
                "Unsupported Linux accelerated rendering backend: {render_backend:?}"
            )),
        }
    }

    pub fn capture(
        &mut self,
        info: &AcceleratedPaintInfo,
        target: NativeCaptureTarget,
    ) -> Result<(), String> {
        self.vulkan_importer.capture(info, target)
    }

    pub fn prepare_publication(&self) -> Result<(), String> {
        self.vulkan_importer.prepare_publication()
    }
}
