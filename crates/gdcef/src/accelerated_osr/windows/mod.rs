mod d3d12;
mod vulkan;

use super::{NativeCaptureTarget, RenderBackend};
use godot::global::godot_print;

pub fn get_godot_gpu_device_ids() -> Option<(u32, u32)> {
    match RenderBackend::detect() {
        RenderBackend::D3D12 => d3d12::get_godot_gpu_device_ids(),
        RenderBackend::Vulkan => vulkan::get_godot_gpu_device_ids(),
        _ => None,
    }
}

pub enum GodotTextureImporter {
    D3D12(d3d12::D3D12TextureImporter),
    Vulkan(Box<vulkan::VulkanTextureImporter>),
}

impl GodotTextureImporter {
    /// Called exclusively during initialization on Godot's rendering thread.
    pub fn new() -> Result<Self, String> {
        let importer = match RenderBackend::detect() {
            RenderBackend::D3D12 => Self::D3D12(d3d12::D3D12TextureImporter::new()?),
            RenderBackend::Vulkan => Self::Vulkan(Box::new(vulkan::VulkanTextureImporter::new()?)),
            _ => return Err("Accelerated OSR requires D3D12 or Vulkan on Windows".into()),
        };
        godot_print!("[AcceleratedOSR] Initialized synchronous native snapshot capture");
        Ok(importer)
    }

    pub fn capture(
        &mut self,
        info: &cef::AcceleratedPaintInfo,
        target: NativeCaptureTarget,
    ) -> Result<(), String> {
        match self {
            Self::D3D12(importer) => importer.capture(info, target),
            Self::Vulkan(importer) => importer.capture(info, target),
        }
    }

    pub fn prepare_publication(&self) -> Result<(), String> {
        match self {
            Self::D3D12(importer) => importer.prepare_publication(),
            Self::Vulkan(importer) => importer.prepare_publication(),
        }
    }
}
