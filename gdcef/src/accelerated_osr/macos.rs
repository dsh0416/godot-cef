use super::RenderBackend;
use cef::AcceleratedPaintInfo;
use godot::classes::RenderingServer;
use godot::classes::rendering_device::DriverResource;
use godot::global::{godot_error, godot_print, godot_warn};
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

pub struct PendingMetalCopy {
    io_surface: *mut c_void,
    width: u32,
    height: u32,
    format: cef::sys::cef_color_type_t,
}

impl Drop for PendingMetalCopy {
    fn drop(&mut self) {
        if !self.io_surface.is_null() {
            unsafe { CFRelease(self.io_surface) };
        }
    }
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRetain(cf: *mut c_void) -> *mut c_void;
    fn CFRelease(cf: *mut c_void);

    fn CFStringCreateWithCString(
        alloc: *const c_void,
        cStr: *const i8,
        encoding: u32,
    ) -> *const c_void;
    fn CFDataGetLength(theData: *const c_void) -> isize;
    fn CFDataGetBytePtr(theData: *const c_void) -> *const u8;
}

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
    pending_copy: Option<PendingMetalCopy>,
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
            pending_copy: None,
        })
    }

    pub fn queue_copy(&mut self, info: &AcceleratedPaintInfo) -> Result<(), String> {
        let io_surface = info.shared_texture_io_surface;
        if io_surface.is_null() {
            return Err("Source IOSurface is null".into());
        }

        let width = info.extra.coded_size.width as u32;
        let height = info.extra.coded_size.height as u32;

        if width == 0 || height == 0 {
            return Err(format!("Invalid source dimensions: {}x{}", width, height));
        }

        // Retain the IOSurface to extend its lifetime beyond the callback
        let retained_surface = unsafe { CFRetain(io_surface) };

        // Replace any existing pending copy (drop the old one, which releases its IOSurface)
        self.pending_copy = Some(PendingMetalCopy {
            io_surface: retained_surface,
            width,
            height,
            format: *info.format.as_ref(),
        });

        Ok(())
    }

    pub fn process_pending_copy(&mut self, dst_rd_rid: Rid) -> Result<(), String> {
        let pending = match self.pending_copy.take() {
            Some(p) => p,
            None => return Ok(()), // Nothing to do
        };

        if !dst_rd_rid.is_valid() {
            return Err("Destination RID is invalid".into());
        }

        // Create Metal texture from IOSurface (source)
        let src_metal_texture = self.metal_importer.import_io_surface(
            pending.io_surface,
            pending.width,
            pending.height,
            pending.format,
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

        self.metal_importer.copy_texture(
            &src_metal_texture,
            dst_texture_ref,
            pending.width,
            pending.height,
        )?;

        // pending is dropped here, which releases the IOSurface
        Ok(())
    }

    pub fn wait_for_copy(&mut self) -> Result<(), String> {
        Ok(())
    }
}

impl Drop for GodotTextureImporter {
    fn drop(&mut self) {
        self.pending_copy = None;

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

// IOKit types and functions for querying GPU registry properties
type IORegistryEntryID = u64;
type IOReturn = i32;
type MachPort = u32;

// IOKit registry iteration options
const K_IO_REGISTRY_ITERATE_RECURSIVELY: u32 = 0x00000001;
const K_IO_REGISTRY_ITERATE_PARENTS: u32 = 0x00000002;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IORegistryEntryIDMatching(entryID: IORegistryEntryID) -> *mut c_void;
    fn IOServiceGetMatchingService(mainPort: MachPort, matching: *mut c_void) -> u32;
    fn IOObjectRelease(object: u32) -> IOReturn;
    // Search for a property in the registry entry and its parents
    fn IORegistryEntrySearchCFProperty(
        entry: u32,
        plane: *const i8,
        key: *const c_void,
        allocator: *const c_void,
        options: u32,
    ) -> *const c_void;
}

const K_CF_STRING_ENCODING_UTF8: u32 = 0x08000100;
const K_IO_SERVICE_PLANE: &[u8] = b"IOService\0";

/// Get a u32 property from an IORegistry entry, searching parents if not found directly.
fn io_registry_search_property_u32(service: u32, key: &str) -> Option<u32> {
    let key_cstr = std::ffi::CString::new(key).ok()?;

    unsafe {
        let cf_key = CFStringCreateWithCString(
            std::ptr::null(),
            key_cstr.as_ptr(),
            K_CF_STRING_ENCODING_UTF8,
        );
        if cf_key.is_null() {
            return None;
        }

        let cf_data = IORegistryEntrySearchCFProperty(
            service,
            K_IO_SERVICE_PLANE.as_ptr() as *const i8,
            cf_key,
            std::ptr::null(),
            K_IO_REGISTRY_ITERATE_RECURSIVELY | K_IO_REGISTRY_ITERATE_PARENTS,
        );
        CFRelease(cf_key as *mut c_void);

        if cf_data.is_null() {
            return None;
        }

        let length = CFDataGetLength(cf_data);
        if length < 4 {
            CFRelease(cf_data as *mut c_void);
            return None;
        }

        let bytes = CFDataGetBytePtr(cf_data);
        if bytes.is_null() {
            CFRelease(cf_data as *mut c_void);
            return None;
        }

        // Read as little-endian u32
        let value = u32::from_le_bytes([*bytes, *bytes.add(1), *bytes.add(2), *bytes.add(3)]);
        CFRelease(cf_data as *mut c_void);

        Some(value)
    }
}

/// Get the GPU vendor and device IDs from Godot's Metal device.
pub fn get_godot_gpu_device_ids() -> Option<(u32, u32)> {
    let mut rd = RenderingServer::singleton().get_rendering_device()?;
    let mtl_device_ptr = rd.get_driver_resource(DriverResource::LOGICAL_DEVICE, Rid::Invalid, 0);

    if mtl_device_ptr == 0 {
        godot_error!("[AcceleratedOSR/Metal] Failed to get Metal device for GPU ID query");
        return None;
    }

    let device: &AnyObject = unsafe { &*(mtl_device_ptr as *const AnyObject) };

    // Get registryID from MTLDevice
    let registry_id: u64 = unsafe { msg_send![device, registryID] };

    if registry_id == 0 {
        godot_error!("[AcceleratedOSR/Metal] Metal device has no registry ID");
        return None;
    }

    // Use IOKit to find the IOService entry and read vendor/device IDs
    unsafe {
        let matching = IORegistryEntryIDMatching(registry_id);
        if matching.is_null() {
            godot_error!("[AcceleratedOSR/Metal] Failed to create IORegistry matching dictionary");
            return None;
        }

        // kIOMasterPortDefault is 0
        let service = IOServiceGetMatchingService(0, matching);
        // matching is consumed by IOServiceGetMatchingService

        if service == 0 {
            godot_error!(
                "[AcceleratedOSR/Metal] No IOService found for registry ID {}",
                registry_id
            );
            return None;
        }

        let vendor_id = io_registry_search_property_u32(service, "vendor-id");
        let device_id = io_registry_search_property_u32(service, "device-id");

        IOObjectRelease(service);

        let name: Option<Retained<AnyObject>> = msg_send![device as &AnyObject, name];
        let name_str = name
            .map(|n| {
                let s: *const std::ffi::c_char = msg_send![&*n, UTF8String];
                if s.is_null() {
                    "Unknown".to_string()
                } else {
                    std::ffi::CStr::from_ptr(s).to_string_lossy().into_owned()
                }
            })
            .unwrap_or_else(|| "Unknown".to_string());

        match (vendor_id, device_id) {
            (Some(vendor), Some(device_id_val)) => {
                godot_print!(
                    "[AcceleratedOSR/Metal] Godot GPU: vendor=0x{:04x}, device=0x{:04x}, name={}",
                    vendor,
                    device_id_val,
                    name_str
                );
                Some((vendor, device_id_val))
            }
            _ => {
                // On Apple Silicon, there are no PCI vendor-id/device-id properties because
                // the GPU is integrated into the SoC, not a discrete PCI device.
                // This is fine - Apple Silicon Macs have only one GPU, so GPU pinning
                // is unnecessary (CEF will always use the same GPU as Godot).
                godot_print!(
                    "[AcceleratedOSR/Metal] GPU '{}' has no PCI vendor/device IDs (expected on Apple Silicon). \
                         GPU pinning not needed on single-GPU systems.",
                    name_str
                );
                None
            }
        }
    }
}

unsafe impl Send for GodotTextureImporter {}
unsafe impl Sync for GodotTextureImporter {}
