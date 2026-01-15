use super::RenderBackend;
use cef::AcceleratedPaintInfo;
use godot::classes::RenderingServer;
use godot::classes::rendering_device::DriverResource;
use godot::global::godot_warn;
use godot::prelude::*;
use objc2::encode::{Encode, Encoding};
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_metal::{
    MTLOrigin, MTLPixelFormat, MTLSize, MTLStorageMode, MTLTextureDescriptor, MTLTextureType,
    MTLTextureUsage,
};
use std::ffi::c_void;

/// Wrapper type for IOSurfaceRef with correct Objective-C type encoding.
/// Metal's `newTextureWithDescriptor:iosurface:plane:` expects `^{__IOSurface=}` encoding.
#[repr(transparent)]
#[derive(Copy, Clone)]
struct IOSurfaceRef(*mut c_void);

unsafe impl Encode for IOSurfaceRef {
    const ENCODING: Encoding = Encoding::Pointer(&Encoding::Struct("__IOSurface", &[]));
}

#[link(name = "IOSurface", kind = "framework")]
unsafe extern "C" {
    fn IOSurfaceGetWidth(buffer: *mut c_void) -> usize;
    fn IOSurfaceGetHeight(buffer: *mut c_void) -> usize;
}

pub struct NativeTextureImporter {
    device: Retained<AnyObject>,
    command_queue: Retained<AnyObject>,
}

impl NativeTextureImporter {
    pub fn new() -> Option<Self> {
        let mut rs = RenderingServer::singleton().get_rendering_device().unwrap();

        let mtl_device_ptr =
            rs.get_driver_resource(DriverResource::LOGICAL_DEVICE, Rid::Invalid, 0);

        if mtl_device_ptr == 0 {
            return None;
        }

        let device: Retained<AnyObject> = unsafe {
            let device_ptr = mtl_device_ptr as *mut AnyObject;
            Retained::retain(device_ptr)?
        };

        let command_queue: Option<Retained<AnyObject>> =
            unsafe { msg_send![&*device, newCommandQueue] };

        let command_queue = match command_queue {
            Some(cq) => cq,
            None => {
                godot_warn!(
                    "Failed to create Metal command queue via newCommandQueue (returned nil)"
                );
                return None;
            }
        };
        Some(Self {
            device,
            command_queue,
        })
    }

    /// Copies from a source Metal texture to a destination Metal texture using blit encoder.
    pub fn copy_texture(
        &self,
        src_texture: &AnyObject,
        dst_texture: &AnyObject,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        let src_origin = MTLOrigin { x: 0, y: 0, z: 0 };
        let src_size = MTLSize {
            width: width as usize,
            height: height as usize,
            depth: 1,
        };
        let dst_origin = MTLOrigin { x: 0, y: 0, z: 0 };

        unsafe {
            let command_buffer_opt: Option<Retained<AnyObject>> =
                msg_send![&*self.command_queue, commandBuffer];
            let command_buffer = match command_buffer_opt {
                Some(cb) => cb,
                None => return Err("Failed to create Metal command buffer".to_string()),
            };
            let blit_encoder_opt: Option<Retained<AnyObject>> =
                msg_send![&*command_buffer, blitCommandEncoder];
            let blit_encoder = match blit_encoder_opt {
                Some(be) => be,
                None => return Err("Failed to create Metal blit command encoder".to_string()),
            };

            let _: () = msg_send![
                &*blit_encoder,
                copyFromTexture: src_texture,
                sourceSlice: 0usize,
                sourceLevel: 0usize,
                sourceOrigin: src_origin,
                sourceSize: src_size,
                toTexture: dst_texture,
                destinationSlice: 0usize,
                destinationLevel: 0usize,
                destinationOrigin: dst_origin
            ];

            let _: () = msg_send![&*blit_encoder, endEncoding];
            let _: () = msg_send![&*command_buffer, commit];
            let _: () = msg_send![&*command_buffer, waitUntilCompleted];
        }

        Ok(())
    }

    pub fn import_io_surface(
        &self,
        io_surface: *mut c_void,
        width: u32,
        height: u32,
        format: cef::sys::cef_color_type_t,
    ) -> Result<Retained<AnyObject>, String> {
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

        // Using sRGB formats to ensure correct gamma handling for web content
        let mtl_pixel_format = match format {
            cef::sys::cef_color_type_t::CEF_COLOR_TYPE_RGBA_8888 => MTLPixelFormat::RGBA8Unorm_sRGB,
            cef::sys::cef_color_type_t::CEF_COLOR_TYPE_BGRA_8888 => MTLPixelFormat::BGRA8Unorm_sRGB,
            _ => MTLPixelFormat::BGRA8Unorm_sRGB,
        };

        unsafe {
            let desc = MTLTextureDescriptor::new();
            desc.setWidth(width as usize);
            desc.setHeight(height as usize);
            desc.setTextureType(MTLTextureType::Type2D);
            desc.setPixelFormat(mtl_pixel_format);
            desc.setUsage(MTLTextureUsage::ShaderRead);
            desc.setStorageMode(MTLStorageMode::Shared);

            let io_surface_ref = IOSurfaceRef(io_surface);
            let texture: Option<Retained<AnyObject>> = msg_send![
                &*self.device,
                newTextureWithDescriptor: &*desc,
                iosurface: io_surface_ref,
                plane: 0usize
            ];

            texture.ok_or_else(|| "Metal texture creation failed".to_string())
        }
    }
}

pub struct GodotTextureImporter {
    metal_importer: NativeTextureImporter,
    current_metal_texture: Option<Retained<AnyObject>>,
    current_texture_rid: Option<Rid>,
}

impl GodotTextureImporter {
    pub fn new() -> Option<Self> {
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

        Some(Self {
            metal_importer,
            current_metal_texture: None,
            current_texture_rid: None,
        })
    }

    pub fn import_and_copy(
        &mut self,
        info: &AcceleratedPaintInfo,
        dst_rd_rid: Rid,
    ) -> Result<u64, String> {
        let io_surface = info.shared_texture_io_surface;
        if io_surface.is_null() {
            return Err("Source IOSurface is null".into());
        }

        let width = info.extra.coded_size.width as u32;
        let height = info.extra.coded_size.height as u32;

        if width == 0 || height == 0 {
            return Err(format!("Invalid source dimensions: {}x{}", width, height));
        }
        if !dst_rd_rid.is_valid() {
            return Err("Destination RID is invalid".into());
        }

        // Create Metal texture from IOSurface (source)
        let src_metal_texture = self.metal_importer.import_io_surface(
            io_surface,
            width,
            height,
            *info.format.as_ref(),
        )?;

        // Get destination Metal texture from Godot's RenderingDevice
        let dst_texture_ptr = {
            let mut rd = RenderingServer::singleton()
                .get_rendering_device()
                .ok_or("Failed to get RenderingDevice")?;

            let texture_ptr = rd.get_driver_resource(DriverResource::TEXTURE, dst_rd_rid, 0);

            if texture_ptr == 0 {
                return Err("Failed to get destination Metal texture handle".into());
            }

            texture_ptr as *mut AnyObject
        };

        // Ensure the destination pointer is suitably aligned for AnyObject before dereferencing.
        let required_align = std::mem::align_of::<AnyObject>();
        if !(dst_texture_ptr as usize).is_multiple_of(required_align) {
            return Err("Destination Metal texture handle is misaligned for AnyObject".into());
        }

        let dst_texture_ref = unsafe { &*dst_texture_ptr };

        // Metal copy is synchronous for now (waitUntilCompleted).
        // TODO: Consider making this async in the future (e.g., to match an async D3D12 implementation once available).
        self.metal_importer
            .copy_texture(&src_metal_texture, dst_texture_ref, width, height)?;

        Ok(0)
    }

    pub fn is_copy_complete(&self, _copy_id: u64) -> bool {
        true
    }

    pub fn wait_for_all_copies(&self) {}
}

impl Drop for GodotTextureImporter {
    fn drop(&mut self) {
        let mut rs = RenderingServer::singleton();
        if let Some(rid) = self.current_texture_rid.take() {
            rs.free_rid(rid);
        }
        self.current_metal_texture.take();
    }
}

pub fn is_supported() -> bool {
    NativeTextureImporter::new().is_some() && RenderBackend::detect().supports_accelerated_osr()
}

unsafe impl Send for GodotTextureImporter {}
unsafe impl Sync for GodotTextureImporter {}
