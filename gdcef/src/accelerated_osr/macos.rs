use super::{NativeHandleTrait, RenderBackend, SharedTextureInfo, TextureImporterTrait};
use cef::AcceleratedPaintInfo;
use godot::classes::rendering_device::DriverResource;
use godot::classes::RenderingServer;
use godot::classes::image::Format as ImageFormat;
use godot::classes::rendering_server::TextureType;
use godot::global::{godot_error, godot_warn};
use godot::prelude::*;
use metal::foreign_types::ForeignType;
use std::ffi::c_void;

const COLOR_SWAP_SHADER: &str = r#"
shader_type canvas_item;

void fragment() {
    vec4 tex_color = texture(TEXTURE, UV);
    COLOR = vec4(tex_color.b, tex_color.g, tex_color.r, tex_color.a);
}
"#;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRetain(cf: *mut c_void) -> *mut c_void;
    fn CFRelease(cf: *mut c_void);
}

#[link(name = "IOSurface", kind = "framework")]
unsafe extern "C" {
    fn IOSurfaceGetWidth(buffer: *mut c_void) -> usize;
    fn IOSurfaceGetHeight(buffer: *mut c_void) -> usize;
}

fn io_surface_retain(io_surface: *mut c_void) -> *mut c_void {
    if io_surface.is_null() {
        return std::ptr::null_mut();
    }
    unsafe { CFRetain(io_surface) }
}

fn io_surface_release(io_surface: *mut c_void) {
    if !io_surface.is_null() {
        unsafe { CFRelease(io_surface) };
    }
}

pub struct NativeHandle {
    io_surface: *mut c_void,
}

impl NativeHandle {
    pub fn as_ptr(&self) -> *mut c_void {
        self.io_surface
    }

    pub fn from_io_surface(io_surface: *mut c_void) -> Self {
        Self {
            io_surface: if io_surface.is_null() {
                std::ptr::null_mut()
            } else {
                io_surface_retain(io_surface)
            },
        }
    }
}

impl Default for NativeHandle {
    fn default() -> Self {
        Self {
            io_surface: std::ptr::null_mut(),
        }
    }
}

impl Clone for NativeHandle {
    fn clone(&self) -> Self {
        Self {
            io_surface: if self.io_surface.is_null() {
                std::ptr::null_mut()
            } else {
                io_surface_retain(self.io_surface)
            },
        }
    }
}

impl Drop for NativeHandle {
    fn drop(&mut self) {
        if !self.io_surface.is_null() {
            io_surface_release(self.io_surface);
            self.io_surface = std::ptr::null_mut();
        }
    }
}

unsafe impl Send for NativeHandle {}
unsafe impl Sync for NativeHandle {}

impl NativeHandleTrait for NativeHandle {
    fn is_valid(&self) -> bool {
        !self.io_surface.is_null()
    }

    fn from_accelerated_paint_info(info: &AcceleratedPaintInfo) -> Self {
        Self::from_io_surface(info.shared_texture_io_surface)
    }
}

pub struct NativeTextureImporter {
    device: metal::Device,
    command_queue: metal::CommandQueue,
}

impl NativeTextureImporter {
    pub fn new() -> Option<Self> {
        let mut rs = RenderingServer::singleton().get_rendering_device().unwrap();
        
        let device = unsafe {
            let mtl_device_ptr = rs.get_driver_resource(DriverResource::LOGICAL_DEVICE, Rid::Invalid, 0);
            metal::Device::from_ptr(mtl_device_ptr as *mut _)
        };

        let command_queue = device.new_command_queue();

        Some(Self { device, command_queue })
    }

    /// Copies from a source Metal texture to a destination Metal texture using blit encoder.
    #[allow(unexpected_cfgs)]
    pub fn copy_texture(
        &self,
        src_texture: *mut objc::runtime::Object,
        dst_texture: *mut objc::runtime::Object,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        use metal::foreign_types::ForeignTypeRef;
        use metal::MTLOrigin;
        use metal::MTLSize;
        use objc::{sel, sel_impl};

        if src_texture.is_null() {
            return Err("Source texture is null".into());
        }
        if dst_texture.is_null() {
            return Err("Destination texture is null".into());
        }

        let command_buffer = self.command_queue.new_command_buffer();
        let blit_encoder = command_buffer.new_blit_command_encoder();

        let src_origin = MTLOrigin { x: 0, y: 0, z: 0 };
        let src_size = MTLSize {
            width: width as u64,
            height: height as u64,
            depth: 1,
        };
        let dst_origin = MTLOrigin { x: 0, y: 0, z: 0 };
        let blit_encoder_ptr = blit_encoder.as_ptr() as *mut objc::runtime::Object;

        unsafe {
            let _: () = objc::msg_send![
                blit_encoder_ptr,
                copyFromTexture:src_texture
                sourceSlice:0usize
                sourceLevel:0usize
                sourceOrigin:src_origin
                sourceSize:src_size
                toTexture:dst_texture
                destinationSlice:0usize
                destinationLevel:0usize
                destinationOrigin:dst_origin
            ];
        }

        blit_encoder.end_encoding();
        command_buffer.commit();
        command_buffer.wait_until_completed();

        Ok(())
    }

    #[allow(unexpected_cfgs)]
    pub fn import_io_surface(
        &self,
        io_surface: *mut c_void,
        width: u32,
        height: u32,
        format: cef::sys::cef_color_type_t,
    ) -> Result<*mut objc::runtime::Object, String> {
        use metal::{MTLPixelFormat, MTLStorageMode, MTLTextureType, MTLTextureUsage};
        use objc::{sel, sel_impl};

        if io_surface.is_null() {
            return Err("IOSurface is null".into());
        }
        if width == 0 || height == 0 {
            return Err(format!("Invalid dimensions: {}x{}", width, height));
        }

        let (ios_width, ios_height) = unsafe {
            (
                IOSurfaceGetWidth(io_surface),
                IOSurfaceGetHeight(io_surface),
            )
        };
        if ios_width != width as usize || ios_height != height as usize {
            godot_warn!(
                "[AcceleratedOSR/macOS] Dimension mismatch: IOSurface {}x{}, expected {}x{}",
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

        Ok(texture)
    }
}

#[allow(unexpected_cfgs)]
fn release_metal_texture(texture: *mut objc::runtime::Object) {
    use objc::{sel, sel_impl};
    if !texture.is_null() {
        unsafe {
            let _: () = objc::msg_send![texture, release];
        }
    }
}

pub struct GodotTextureImporter {
    metal_importer: NativeTextureImporter,
    current_metal_texture: Option<*mut objc::runtime::Object>,
    current_texture_rid: Option<Rid>,
    color_swap_shader: Option<Rid>,
    color_swap_material: Option<Rid>,
}

impl TextureImporterTrait for GodotTextureImporter {
    type Handle = NativeHandle;

    fn new() -> Option<Self> {
        let metal_importer = NativeTextureImporter::new()?;
        let render_backend = RenderBackend::detect();

        if !render_backend.supports_accelerated_osr() {
            godot_warn!(
                "[AcceleratedOSR/macOS] Render backend {:?} does not support accelerated OSR. \
                 Metal backend is required on macOS.",
                render_backend
            );
            return None;
        }

        let mut rs = RenderingServer::singleton();
        let shader_rid = rs.shader_create();
        rs.shader_set_code(shader_rid, COLOR_SWAP_SHADER);
        let material_rid = rs.material_create();
        rs.material_set_shader(material_rid, shader_rid);

        Some(Self {
            metal_importer,
            current_metal_texture: None,
            current_texture_rid: None,
            color_swap_shader: Some(shader_rid),
            color_swap_material: Some(material_rid),
        })
    }

    fn import_texture(&mut self, texture_info: &SharedTextureInfo<Self::Handle>) -> Option<Rid> {
        let io_surface = texture_info.native_handle().as_ptr();
        if io_surface.is_null() || texture_info.width == 0 || texture_info.height == 0 {
            return None;
        }

        let metal_texture = self
            .metal_importer
            .import_io_surface(
                io_surface,
                texture_info.width,
                texture_info.height,
                texture_info.format,
            )
            .map_err(|e| godot_error!("[AcceleratedOSR/macOS] Metal import failed: {}", e))
            .ok()?;

        if let Some(old_rid) = self.current_texture_rid.take() {
            RenderingServer::singleton().free_rid(old_rid);
        }

        if let Some(old) = self.current_metal_texture.take() {
            release_metal_texture(old);
        }

        self.current_metal_texture = Some(metal_texture);

        let (native_handle, texture_rid) = {
            let handle = metal_texture as u64;
            let rid = RenderingServer::singleton().texture_create_from_native_handle(
                TextureType::TYPE_2D,
                ImageFormat::RGBA8,
                handle,
                texture_info.width as i32,
                texture_info.height as i32,
                1,
            );
            (handle, rid)
        };

        if !texture_rid.is_valid() {
            godot_error!(
                "[AcceleratedOSR/macOS] Created texture RID is invalid (handle: {})",
                native_handle
            );
            return None;
        }

        self.current_texture_rid = Some(texture_rid);
        Some(texture_rid)
    }

    fn copy_texture(
        &mut self,
        src_info: &SharedTextureInfo<Self::Handle>,
        dst_rd_rid: Rid,
    ) -> Result<(), String> {
        let io_surface = src_info.native_handle().as_ptr();
        if io_surface.is_null() {
            return Err("Source IOSurface is null".into());
        }
        if src_info.width == 0 || src_info.height == 0 {
            return Err(format!(
                "Invalid source dimensions: {}x{}",
                src_info.width, src_info.height
            ));
        }
        if !dst_rd_rid.is_valid() {
            return Err("Destination RID is invalid".into());
        }

        // Create Metal texture from IOSurface (source)
        let src_metal_texture = self
            .metal_importer
            .import_io_surface(io_surface, src_info.width, src_info.height, src_info.format)?;

        // Get destination Metal texture from Godot's RenderingDevice
        let dst_metal_texture = {
            let mut rd = RenderingServer::singleton()
                .get_rendering_device()
                .ok_or("Failed to get RenderingDevice")?;
            
            let texture_ptr = rd.get_driver_resource(
                DriverResource::TEXTURE,
                dst_rd_rid,
                0,
            );
            
            if texture_ptr == 0 {
                release_metal_texture(src_metal_texture);
                return Err("Failed to get destination Metal texture handle".into());
            }
            
            texture_ptr as *mut objc::runtime::Object
        };

        // Perform the GPU copy
        let result = self.metal_importer.copy_texture(
            src_metal_texture,
            dst_metal_texture,
            src_info.width,
            src_info.height,
        );

        // Clean up source texture (it was created just for this copy)
        release_metal_texture(src_metal_texture);

        result
    }

    fn get_color_swap_material(&self) -> Option<Rid> {
        self.color_swap_material
    }
}

impl Drop for GodotTextureImporter {
    fn drop(&mut self) {
        let mut rs = RenderingServer::singleton();
        if let Some(rid) = self.current_texture_rid.take() {
            rs.free_rid(rid);
        }
        if let Some(tex) = self.current_metal_texture.take() {
            release_metal_texture(tex);
        }
        if let Some(rid) = self.color_swap_material.take() {
            rs.free_rid(rid);
        }
        if let Some(rid) = self.color_swap_shader.take() {
            rs.free_rid(rid);
        }
    }
}

pub fn is_supported() -> bool {
    NativeTextureImporter::new().is_some() && RenderBackend::detect().supports_accelerated_osr()
}
