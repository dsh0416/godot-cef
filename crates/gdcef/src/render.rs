//! Rendering utilities for CEF texture management.
//!
//! This module provides helper functions for creating and managing RenderingDevice
//! textures used for GPU-accelerated off-screen rendering.

use crate::error::{CefError, CefResult};
use godot::classes::RenderingServer;
use godot::classes::rendering_device::{
    DataFormat, TextureSamples, TextureType as RdTextureType, TextureUsageBits,
};
use godot::prelude::*;

/// Only call on Godot's render thread. The caller owns the returned RD resource.
pub(crate) fn create_rd_texture_rid(
    width: i32,
    height: i32,
    data_format: DataFormat,
) -> CefResult<Rid> {
    let width = width.max(1) as i64;
    let height = height.max(1) as i64;

    let mut rd = RenderingServer::singleton()
        .get_rendering_device()
        .ok_or_else(|| CefError::GpuDeviceError("Failed to get RenderingDevice".to_string()))?;

    let mut format = godot::classes::RdTextureFormat::new_gd();
    format.set_format(data_format);
    // Texture2DRD creates both linear and sRGB views of its backing texture.
    // Both formats must be declared even when the resource itself is sRGB.
    format.add_shareable_format(data_format);
    match data_format {
        DataFormat::B8G8R8A8_SRGB => {
            format.add_shareable_format(DataFormat::B8G8R8A8_UNORM);
        }
        DataFormat::R8G8B8A8_SRGB => {
            format.add_shareable_format(DataFormat::R8G8B8A8_UNORM);
        }
        _ => {}
    }
    format.set_width(width as u32);
    format.set_height(height as u32);
    format.set_depth(1);
    format.set_array_layers(1);
    format.set_mipmaps(1);
    format.set_texture_type(RdTextureType::TYPE_2D);
    format.set_samples(TextureSamples::SAMPLES_1);
    format.set_usage_bits(
        TextureUsageBits::SAMPLING_BIT
            | TextureUsageBits::CAN_UPDATE_BIT
            | TextureUsageBits::CAN_COPY_TO_BIT
            | TextureUsageBits::CAN_COPY_FROM_BIT,
    );

    let rd_texture_rid = rd.texture_create(&format, &godot::classes::RdTextureView::new_gd());

    if !rd_texture_rid.is_valid() {
        return Err(CefError::TextureOperationFailed(format!(
            "Failed to create RenderingDevice texture {}x{}",
            width, height
        )));
    }

    // One initial transparent upload avoids sRGB RTV/UAV clear restrictions on
    // Godot's D3D12 backend. Browser frame pixels still stay entirely on the GPU.
    let transparent = PackedByteArray::from(vec![0; (width * height * 4) as usize].as_slice());
    let error = rd.texture_update(rd_texture_rid, 0, &transparent);
    if error != godot::global::Error::OK {
        rd.free_rid(rd_texture_rid);
        return Err(CefError::TextureOperationFailed(format!(
            "Failed to initialize display texture: {error:?}"
        )));
    }
    Ok(rd_texture_rid)
}

pub fn free_rd_texture(rd_texture_rid: Rid) {
    if rd_texture_rid.is_valid() {
        on_render_thread(move || {
            if let Some(mut rd) = RenderingServer::singleton().get_rendering_device() {
                rd.free_rid(rd_texture_rid);
            }
        });
    }
}

pub(crate) fn on_render_thread<F: FnOnce() + Send + Sync + 'static>(work: F) {
    let mut work = Some(work);
    let callable = Callable::from_sync_fn("gdcef_render", move |_: &[&Variant]| {
        if let Some(work) = work.take() {
            work();
        }
    });
    RenderingServer::singleton().call_on_render_thread(&callable);
}

/// Initialization only: waits for CPU work on the rendering server, never for a
/// future draw. Do not call while holding shared render state or from CEF paint.
pub(crate) fn on_render_thread_sync<T, F>(work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + Sync + 'static,
{
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    on_render_thread(move || {
        let _ = sender.send(work());
    });
    receiver
        .recv()
        .map_err(|_| "Rendering server discarded initialization work".to_string())
}
