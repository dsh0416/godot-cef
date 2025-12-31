//! Accelerated Off-Screen Rendering (OSR) for macOS
//!
//! This module implements GPU-accelerated texture sharing between CEF and Godot
//! using IOSurface on macOS. When enabled, CEF renders to an IOSurface which is
//! imported as a Metal texture and shared directly with Godot via
//! `RenderingServer.texture_create_from_native_handle()`.
//!
//! ## Architecture (Zero-Copy)
//!
//! ```text
//! CEF (Chromium) -> IOSurface -> Metal Texture -> Godot RenderingServer
//! ```
//!
//! This approach avoids any CPU copies - the IOSurface rendered by CEF's GPU
//! compositor is directly imported into Godot's rendering pipeline.

use cef::{AcceleratedPaintInfo, PaintElementType};
use godot::classes::image::Format as ImageFormat;
use godot::classes::rendering_server::TextureType;
use godot::classes::RenderingServer;
use godot::global::{godot_error, godot_print, godot_warn};
use godot::prelude::*;
use std::ffi::c_void;
use std::sync::{Arc, Mutex};

#[cfg(target_os = "macos")]
const COLOR_SWAP_SHADER: &str = r#"
shader_type canvas_item;

void fragment() {
    vec4 tex_color = texture(TEXTURE, UV);
    COLOR = vec4(tex_color.b, tex_color.g, tex_color.r, tex_color.a);
}
"#;

#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRetain(cf: *mut c_void) -> *mut c_void;
    fn CFRelease(cf: *mut c_void);
}

#[cfg(target_os = "macos")]
#[link(name = "IOSurface", kind = "framework")]
unsafe extern "C" {
    fn IOSurfaceGetWidth(buffer: *mut c_void) -> usize;
    fn IOSurfaceGetHeight(buffer: *mut c_void) -> usize;
}

#[cfg(target_os = "macos")]
fn io_surface_retain(io_surface: *mut c_void) -> *mut c_void {
    if io_surface.is_null() {
        return std::ptr::null_mut();
    }
    unsafe { CFRetain(io_surface) }
}

#[cfg(target_os = "macos")]
fn io_surface_release(io_surface: *mut c_void) {
    if !io_surface.is_null() {
        unsafe { CFRelease(io_surface) };
    }
}

/// Shared IOSurface texture info from CEF with reference counting.
pub struct SharedTextureInfo {
    io_surface: *mut c_void,
    pub width: u32,
    pub height: u32,
    pub format: cef::sys::cef_color_type_t,
    pub dirty: bool,
    pub frame_count: u64,
}

impl SharedTextureInfo {
    pub fn io_surface(&self) -> *mut c_void {
        self.io_surface
    }

    #[cfg(target_os = "macos")]
    pub fn set_io_surface(&mut self, new_surface: *mut c_void) {
        if !self.io_surface.is_null() {
            io_surface_release(self.io_surface);
        }
        self.io_surface = if new_surface.is_null() {
            std::ptr::null_mut()
        } else {
            io_surface_retain(new_surface)
        };
    }

    #[cfg(not(target_os = "macos"))]
    pub fn set_io_surface(&mut self, new_surface: *mut c_void) {
        self.io_surface = new_surface;
    }
}

impl Default for SharedTextureInfo {
    fn default() -> Self {
        Self {
            io_surface: std::ptr::null_mut(),
            width: 0,
            height: 0,
            format: cef::sys::cef_color_type_t::CEF_COLOR_TYPE_BGRA_8888,
            dirty: false,
            frame_count: 0,
        }
    }
}

impl Clone for SharedTextureInfo {
    fn clone(&self) -> Self {
        #[cfg(target_os = "macos")]
        let io_surface = if self.io_surface.is_null() {
            std::ptr::null_mut()
        } else {
            io_surface_retain(self.io_surface)
        };
        #[cfg(not(target_os = "macos"))]
        let io_surface = self.io_surface;

        Self {
            io_surface,
            width: self.width,
            height: self.height,
            format: self.format,
            dirty: self.dirty,
            frame_count: self.frame_count,
        }
    }
}

#[cfg(target_os = "macos")]
impl Drop for SharedTextureInfo {
    fn drop(&mut self) {
        if !self.io_surface.is_null() {
            io_surface_release(self.io_surface);
            self.io_surface = std::ptr::null_mut();
        }
    }
}

unsafe impl Send for SharedTextureInfo {}
unsafe impl Sync for SharedTextureInfo {}

#[cfg(target_os = "macos")]
pub struct MetalTextureImporter {
    device: metal::Device,
}

#[cfg(target_os = "macos")]
impl MetalTextureImporter {
    pub fn new() -> Option<Self> {
        let device = metal::Device::system_default()?;
        godot_print!("[AcceleratedOSR] Metal device: {}", device.name());
        Some(Self { device })
    }

    #[allow(unexpected_cfgs)]
    pub fn import_io_surface(
        &self,
        io_surface: *mut c_void,
        width: u32,
        height: u32,
        format: cef::sys::cef_color_type_t,
    ) -> Result<u64, String> {
        use metal::{MTLPixelFormat, MTLStorageMode, MTLTextureType, MTLTextureUsage};
        use objc::{sel, sel_impl};

        if io_surface.is_null() {
            return Err("IOSurface is null".into());
        }
        if width == 0 || height == 0 {
            return Err(format!("Invalid dimensions: {}x{}", width, height));
        }

        // Validate IOSurface dimensions
        let (ios_width, ios_height) = unsafe {
            (
                IOSurfaceGetWidth(io_surface),
                IOSurfaceGetHeight(io_surface),
            )
        };
        if ios_width != width as usize || ios_height != height as usize {
            godot_warn!(
                "[AcceleratedOSR] Dimension mismatch: IOSurface {}x{}, expected {}x{}",
                ios_width,
                ios_height,
                width,
                height
            );
        }

        let mtl_pixel_format = match format {
            cef::sys::cef_color_type_t::CEF_COLOR_TYPE_RGBA_8888 => MTLPixelFormat::RGBA8Unorm,
            _ => MTLPixelFormat::BGRA8Unorm,
        };

        let desc = metal::TextureDescriptor::new();
        desc.set_width(width as u64);
        desc.set_height(height as u64);
        desc.set_texture_type(MTLTextureType::D2);
        desc.set_pixel_format(mtl_pixel_format);
        desc.set_usage(MTLTextureUsage::ShaderRead);
        desc.set_storage_mode(MTLStorageMode::Managed);

        let texture: *mut objc::runtime::Object = unsafe {
            objc::msg_send![
                self.device.as_ref(),
                newTextureWithDescriptor:desc.as_ref()
                iosurface:io_surface
                plane:0usize
            ]
        };

        if texture.is_null() {
            return Err("Metal texture creation failed".into());
        }

        Ok(texture as u64)
    }
}

/// Handler for CEF's on_accelerated_paint callback.
pub struct AcceleratedRenderHandler {
    pub texture_info: Arc<Mutex<SharedTextureInfo>>,
    pub device_scale_factor: Arc<Mutex<f32>>,
    pub size: Arc<Mutex<winit::dpi::PhysicalSize<f32>>>,
    pub cursor_type: Arc<Mutex<cef_app::CursorType>>,
}

impl Clone for AcceleratedRenderHandler {
    fn clone(&self) -> Self {
        Self {
            texture_info: self.texture_info.clone(),
            device_scale_factor: self.device_scale_factor.clone(),
            size: self.size.clone(),
            cursor_type: self.cursor_type.clone(),
        }
    }
}

impl AcceleratedRenderHandler {
    pub fn new(device_scale_factor: f32, size: winit::dpi::PhysicalSize<f32>) -> Self {
        Self {
            texture_info: Arc::new(Mutex::new(SharedTextureInfo::default())),
            device_scale_factor: Arc::new(Mutex::new(device_scale_factor)),
            size: Arc::new(Mutex::new(size)),
            cursor_type: Arc::new(Mutex::new(cef_app::CursorType::default())),
        }
    }

    pub fn on_accelerated_paint(&self, type_: PaintElementType, info: Option<&AcceleratedPaintInfo>) {
        let Some(info) = info else {
            return;
        };

        // Only handle main frame (PET_VIEW)
        if type_ != PaintElementType::default() {
            return;
        }

        if let Ok(mut tex_info) = self.texture_info.lock() {
            tex_info.frame_count += 1;
            tex_info.set_io_surface(info.shared_texture_io_surface);
            tex_info.width = info.extra.coded_size.width as u32;
            tex_info.height = info.extra.coded_size.height as u32;
            tex_info.format = *info.format.as_ref();
            tex_info.dirty = true;
        } else {
            godot_error!("[AcceleratedOSR] Failed to lock texture_info");
        }
    }

    pub fn get_texture_info(&self) -> Arc<Mutex<SharedTextureInfo>> {
        self.texture_info.clone()
    }

    pub fn get_size(&self) -> Arc<Mutex<winit::dpi::PhysicalSize<f32>>> {
        self.size.clone()
    }

    pub fn get_device_scale_factor(&self) -> Arc<Mutex<f32>> {
        self.device_scale_factor.clone()
    }

    pub fn get_cursor_type(&self) -> Arc<Mutex<cef_app::CursorType>> {
        self.cursor_type.clone()
    }
}

/// Imports IOSurface textures into Godot via native handle API.
#[cfg(target_os = "macos")]
pub struct GodotTextureImporter {
    metal_importer: MetalTextureImporter,
    current_texture_rid: Option<Rid>,
    current_metal_texture: Option<*mut objc::runtime::Object>,
    color_swap_shader: Option<Rid>,
    color_swap_material: Option<Rid>,
}

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
fn release_metal_texture(texture: *mut objc::runtime::Object) {
    use objc::{sel, sel_impl};
    if !texture.is_null() {
        unsafe {
            let _: () = objc::msg_send![texture, release];
        }
    }
}

#[cfg(target_os = "macos")]
impl GodotTextureImporter {
    pub fn new() -> Option<Self> {
        let metal_importer = MetalTextureImporter::new()?;
        let mut rs = RenderingServer::singleton();

        let shader_rid = rs.shader_create();
        rs.shader_set_code(shader_rid, COLOR_SWAP_SHADER);
        let material_rid = rs.material_create();
        rs.material_set_shader(material_rid, shader_rid);

        Some(Self {
            metal_importer,
            current_texture_rid: None,
            current_metal_texture: None,
            color_swap_shader: Some(shader_rid),
            color_swap_material: Some(material_rid),
        })
    }

    pub fn get_color_swap_material(&self) -> Option<Rid> {
        self.color_swap_material
    }

    pub fn import_texture(&mut self, texture_info: &SharedTextureInfo) -> Option<Rid> {
        let io_surface = texture_info.io_surface();
        if io_surface.is_null() || texture_info.width == 0 || texture_info.height == 0 {
            return None;
        }

        let metal_handle = self
            .metal_importer
            .import_io_surface(
                io_surface,
                texture_info.width,
                texture_info.height,
                texture_info.format,
            )
            .map_err(|e| godot_error!("[AcceleratedOSR] Metal import failed: {}", e))
            .ok()?;

        let metal_texture = metal_handle as *mut objc::runtime::Object;

        if let Some(old_rid) = self.current_texture_rid.take() {
            RenderingServer::singleton().free_rid(old_rid);
        }
        if let Some(old_metal) = self.current_metal_texture.take() {
            release_metal_texture(old_metal);
        }

        let texture_rid = RenderingServer::singleton().texture_create_from_native_handle(
            TextureType::TYPE_2D,
            ImageFormat::RGBA8,
            metal_handle,
            texture_info.width as i32,
            texture_info.height as i32,
            1,
        );

        if !texture_rid.is_valid() {
            release_metal_texture(metal_texture);
            godot_error!("[AcceleratedOSR] Created texture RID is invalid");
            return None;
        }

        self.current_texture_rid = Some(texture_rid);
        self.current_metal_texture = Some(metal_texture);

        Some(texture_rid)
    }
}

#[cfg(target_os = "macos")]
impl Drop for GodotTextureImporter {
    fn drop(&mut self) {
        let mut rs = RenderingServer::singleton();
        if let Some(rid) = self.current_texture_rid.take() {
            rs.free_rid(rid);
        }
        if let Some(metal) = self.current_metal_texture.take() {
            release_metal_texture(metal);
        }
        if let Some(rid) = self.color_swap_material.take() {
            rs.free_rid(rid);
        }
        if let Some(rid) = self.color_swap_shader.take() {
            rs.free_rid(rid);
        }
    }
}

pub fn is_accelerated_osr_supported() -> bool {
    #[cfg(target_os = "macos")]
    {
        MetalTextureImporter::new().is_some()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}
