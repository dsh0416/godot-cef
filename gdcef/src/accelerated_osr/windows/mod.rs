mod d3d12;
mod vulkan;

use super::RenderBackend;
use godot::classes::RenderingServer;
use godot::global::{godot_print, godot_warn};
use godot::prelude::*;

use d3d12::D3D12TextureImporter;
use vulkan::VulkanTextureImporter;

pub fn get_godot_adapter_luid() -> Option<(i32, u32)> {
    let backend = RenderBackend::detect();
    match backend {
        RenderBackend::D3D12 => d3d12::get_godot_adapter_luid(),
        RenderBackend::Vulkan => vulkan::get_godot_adapter_luid(),
        _ => {
            godot_warn!(
                "[AcceleratedOSR/Windows] Cannot get adapter LUID for backend {:?}",
                backend
            );
            None
        }
    }
}

pub struct GodotTextureImporter {
    backend: TextureImporterBackend,
    current_texture_rid: Option<Rid>,
}

enum TextureImporterBackend {
    D3D12(D3D12TextureImporter),
    Vulkan(VulkanTextureImporter),
}

impl GodotTextureImporter {
    pub fn new() -> Option<Self> {
        let render_backend = RenderBackend::detect();

        if !render_backend.supports_accelerated_osr() {
            godot_warn!(
                "[AcceleratedOSR/Windows] Render backend {:?} does not support accelerated OSR. \
                 D3D12 or Vulkan backend is required on Windows.",
                render_backend
            );
            return None;
        }

        let backend = match render_backend {
            RenderBackend::D3D12 => {
                let importer = D3D12TextureImporter::new()?;
                godot_print!("[AcceleratedOSR/Windows] Using D3D12 backend for texture import");
                TextureImporterBackend::D3D12(importer)
            }
            RenderBackend::Vulkan => {
                let importer = VulkanTextureImporter::new()?;
                godot_print!("[AcceleratedOSR/Windows] Using Vulkan backend for texture import");
                TextureImporterBackend::Vulkan(importer)
            }
            _ => {
                godot_warn!(
                    "[AcceleratedOSR/Windows] Unexpected backend {:?}",
                    render_backend
                );
                return None;
            }
        };

        Some(Self {
            backend,
            current_texture_rid: None,
        })
    }

    pub fn queue_copy(&mut self, info: &cef::AcceleratedPaintInfo) -> Result<(), String> {
        match &mut self.backend {
            TextureImporterBackend::D3D12(importer) => importer.queue_copy(info),
            TextureImporterBackend::Vulkan(importer) => importer.queue_copy(info),
        }
    }

    pub fn process_pending_copy(&mut self, dst_rd_rid: Rid) -> Result<(), String> {
        match &mut self.backend {
            TextureImporterBackend::D3D12(importer) => importer.process_pending_copy(dst_rd_rid),
            TextureImporterBackend::Vulkan(importer) => importer.process_pending_copy(dst_rd_rid),
        }
    }

    pub fn wait_for_copy(&mut self) -> Result<(), String> {
        match &mut self.backend {
            TextureImporterBackend::D3D12(importer) => importer.wait_for_copy(),
            TextureImporterBackend::Vulkan(importer) => importer.wait_for_copy(),
        }
    }
}

impl Drop for GodotTextureImporter {
    fn drop(&mut self) {
        if let Some(rid) = self.current_texture_rid.take() {
            RenderingServer::singleton().free_rid(rid);
        }
    }
}

pub fn is_supported() -> bool {
    let backend = RenderBackend::detect();
    if !backend.supports_accelerated_osr() {
        return false;
    }

    match backend {
        RenderBackend::D3D12 => D3D12TextureImporter::new().is_some(),
        RenderBackend::Vulkan => VulkanTextureImporter::new().is_some(),
        _ => false,
    }
}

unsafe impl Send for GodotTextureImporter {}
unsafe impl Sync for GodotTextureImporter {}
