//! Rendering utilities for CEF texture management.
//!
//! This module provides helper functions for creating and managing RenderingDevice
//! textures used for GPU-accelerated off-screen rendering.

use crate::error::{CefError, CefResult};
use godot::classes::rendering_device::{
    DataFormat, TextureSamples, TextureType as RdTextureType, TextureUsageBits,
};
use godot::classes::{RenderingServer, Texture2Drd};
use godot::prelude::*;

/// Creates a RenderingDevice texture for CEF rendering.
pub fn create_rd_texture(width: i32, height: i32) -> CefResult<(Rid, Gd<Texture2Drd>)> {
    let width = width.max(1) as i64;
    let height = height.max(1) as i64;

    let mut rd = RenderingServer::singleton()
        .get_rendering_device()
        .ok_or_else(|| CefError::GpuDeviceError("Failed to get RenderingDevice".to_string()))?;

    let mut format = godot::classes::RdTextureFormat::new_gd();
    format.add_shareable_format(DataFormat::B8G8R8A8_UNORM);
    format.add_shareable_format(DataFormat::B8G8R8A8_SRGB);
    format.set_format(DataFormat::B8G8R8A8_SRGB);
    format.set_width(width as u32);
    format.set_height(height as u32);
    format.set_depth(1);
    format.set_array_layers(1);
    format.set_mipmaps(1);
    format.set_texture_type(RdTextureType::TYPE_2D);
    format.set_samples(TextureSamples::SAMPLES_1);
    format.set_usage_bits(TextureUsageBits::SAMPLING_BIT | TextureUsageBits::CAN_COPY_TO_BIT);

    let rd_texture_rid = rd.texture_create(&format, &godot::classes::RdTextureView::new_gd());

    if !rd_texture_rid.is_valid() {
        return Err(CefError::TextureOperationFailed(format!(
            "Failed to create RenderingDevice texture {}x{}",
            width, height
        )));
    }

    let mut texture_2d_rd = Texture2Drd::new_gd();
    texture_2d_rd.set_texture_rd_rid(rd_texture_rid);

    Ok((rd_texture_rid, texture_2d_rd))
}

pub fn free_rd_texture(rd_texture_rid: Rid) {
    if rd_texture_rid.is_valid()
        && let Some(mut rd) = RenderingServer::singleton().get_rendering_device()
    {
        rd.free_rid(rd_texture_rid);
    }
}
