//! Linux-specific accelerated OSR implementation.
//!
//! On Linux, we use Vulkan with DMA-BUF external memory extensions to import
//! shared textures from CEF's compositor process.

mod vulkan;

use super::RenderBackend;
use cef::AcceleratedPaintInfo;
use godot::global::{godot_print, godot_warn};
use godot::prelude::*;

pub fn get_godot_device_uuid() -> Option<[u8; 16]> {
    vulkan::get_godot_device_uuid()
}

pub struct GodotTextureImporter {
    vulkan_importer: vulkan::VulkanTextureImporter,
}

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

        match render_backend {
            RenderBackend::Vulkan => {
                let vulkan_importer = vulkan::VulkanTextureImporter::new()?;
                godot_print!("[AcceleratedOSR/Linux] Using Vulkan backend with DMA-BUF");
                Some(Self { vulkan_importer })
            }
            _ => {
                godot_warn!(
                    "[AcceleratedOSR/Linux] Unsupported render backend: {:?}",
                    render_backend
                );
                None
            }
        }
    }

    pub fn import_and_copy(
        &mut self,
        info: &AcceleratedPaintInfo,
        dst_rd_rid: Rid,
    ) -> Result<(), String> {
        self.vulkan_importer.import_and_copy(info, dst_rd_rid)
    }
}

pub fn is_supported() -> bool {
    let render_backend = RenderBackend::detect();
    render_backend == RenderBackend::Vulkan
}

unsafe impl Send for GodotTextureImporter {}
unsafe impl Sync for GodotTextureImporter {}
