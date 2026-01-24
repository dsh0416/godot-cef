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

    /// Queue a copy operation for deferred processing.
    /// This method returns immediately after duplicating the file descriptors.
    /// Call `process_pending_copy()` later to actually perform the GPU work.
    pub fn queue_copy(&mut self, info: &AcceleratedPaintInfo) -> Result<(), String> {
        self.vulkan_importer.queue_copy(info)
    }

    /// Returns true if there's a pending copy operation waiting to be processed.
    #[allow(dead_code)]
    pub fn has_pending_copy(&self) -> bool {
        self.vulkan_importer.has_pending_copy()
    }

    /// Process the pending copy operation. This does the actual GPU work.
    /// Should be called from Godot's main loop, not from CEF callbacks.
    /// The dst_rd_rid is passed at processing time so resize can update the destination.
    pub fn process_pending_copy(&mut self, dst_rd_rid: Rid) -> Result<(), String> {
        self.vulkan_importer.process_pending_copy(dst_rd_rid)
    }

    /// Wait for any in-flight copy to complete.
    #[allow(dead_code)]
    pub fn wait_for_copy(&mut self) -> Result<(), String> {
        self.vulkan_importer.wait_for_copy()
    }
}

pub fn is_supported() -> bool {
    let render_backend = RenderBackend::detect();
    render_backend == RenderBackend::Vulkan
}

unsafe impl Send for GodotTextureImporter {}
unsafe impl Sync for GodotTextureImporter {}
