//! Metal snapshot capture on Godot's own command queue.
//!
//! Godot 4.6 creates RD textures with MTLResourceHazardTrackingModeTracked:
//! <https://github.com/godotengine/godot/blob/89cea143987d564363e15d207438530651d943ac/drivers/metal/rendering_device_driver_metal.mm#L312-L327>
//! Using that same thread-safe MTLCommandQueue lets Metal track dependencies
//! across our blit and Godot's command buffers. The pool separately guarantees
//! the target has no outstanding readers. waitUntilCompleted ends CEF's lease
//! before returning; retaining an IOSurface alone would not preserve its pixels.

use super::{NativeCaptureTarget, SnapshotFormat};
use cef::AcceleratedPaintInfo;
use godot::classes::RenderingServer;
use godot::classes::rendering_device::DriverResource;
use godot::global::{godot_error, godot_print};
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

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *mut c_void);
    fn CFStringCreateWithCString(
        alloc: *const c_void,
        cStr: *const i8,
        encoding: u32,
    ) -> *const c_void;
    fn CFDataGetLength(theData: *const c_void) -> isize;
    fn CFDataGetBytePtr(theData: *const c_void) -> *const u8;
}

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

pub struct GodotTextureImporter {
    device: Retained<AnyObject>,
    command_queue: Retained<AnyObject>,
}

impl GodotTextureImporter {
    /// Rendering thread only. Retain native objects, never claim ownership of a
    /// borrowed Godot reference. MTLCommandQueue itself supports concurrent use.
    pub fn new() -> Result<Self, String> {
        let rd = RenderingServer::singleton()
            .get_rendering_device()
            .ok_or("Metal RenderingDevice unavailable")?;
        let device_ptr = rd.get_driver_resource(DriverResource::LOGICAL_DEVICE, Rid::Invalid, 0);
        let queue_ptr = rd.get_driver_resource(DriverResource::COMMAND_QUEUE, Rid::Invalid, 0);
        let device = unsafe { Retained::retain(device_ptr as *mut AnyObject) }
            .ok_or("Godot did not expose its Metal device")?;
        let command_queue = unsafe { Retained::retain(queue_ptr as *mut AnyObject) }
            .ok_or("Godot did not expose its Metal command queue")?;
        Ok(Self {
            device,
            command_queue,
        })
    }

    pub fn capture(
        &mut self,
        info: &AcceleratedPaintInfo,
        target: NativeCaptureTarget,
    ) -> Result<(), String> {
        let surface = info.shared_texture_io_surface;
        if surface.is_null() || target.native_handle == 0 || target.width == 0 || target.height == 0
        {
            return Err("Invalid Metal capture source or target".into());
        }
        if SnapshotFormat::from_cef(info)? != target.format {
            return Err("Metal snapshot format does not match CEF's frame".into());
        }
        let (width, height) = unsafe { (IOSurfaceGetWidth(surface), IOSurfaceGetHeight(surface)) };
        if width < target.width as usize || height < target.height as usize {
            return Err("CEF IOSurface is smaller than its coded frame dimensions".into());
        }
        let destination = unsafe { Retained::retain(target.native_handle as *mut AnyObject) }
            .ok_or("Metal snapshot texture is null")?;
        // Automatic hazard tracking is only guaranteed for tracked resources on
        // one queue. Check it instead of relying on future engine defaults.
        let tracked: usize = unsafe { msg_send![&*destination, hazardTrackingMode] };
        let target_width: usize = unsafe { msg_send![&*destination, width] };
        let target_height: usize = unsafe { msg_send![&*destination, height] };
        if tracked != 2
            || target_width != target.width as usize
            || target_height != target.height as usize
        {
            return Err(
                "Metal snapshot requires a tracked texture with matching dimensions".into(),
            );
        }
        unsafe {
            let desc = MTLTextureDescriptor::new();
            desc.setWidth(width);
            desc.setHeight(height);
            desc.setTextureType(MTLTextureType::Type2D);
            desc.setPixelFormat(match target.format {
                SnapshotFormat::Bgra8 => MTLPixelFormat::BGRA8Unorm_sRGB,
                SnapshotFormat::Rgba8 => MTLPixelFormat::RGBA8Unorm_sRGB,
            });
            desc.setUsage(MTLTextureUsage::ShaderRead);
            desc.setStorageMode(MTLStorageMode::Shared);
            let _: () = msg_send![&*desc, setHazardTrackingMode: 2usize];
            let source: Option<Retained<AnyObject>> = msg_send![&*self.device,
                newTextureWithDescriptor: &*desc, iosurface: IOSurfaceRef(surface), plane: 0usize];
            let source = source.ok_or("Cannot open CEF IOSurface as a Metal texture")?;
            let command: Option<Retained<AnyObject>> =
                msg_send![&*self.command_queue, commandBuffer];
            let command = command.ok_or("Cannot allocate Metal capture command buffer")?;
            let blit: Option<Retained<AnyObject>> = msg_send![&*command, blitCommandEncoder];
            let blit = blit.ok_or("Cannot allocate Metal capture blit encoder")?;
            let origin = MTLOrigin { x: 0, y: 0, z: 0 };
            let size = MTLSize {
                width: target.width as usize,
                height: target.height as usize,
                depth: 1,
            };
            let _: () = msg_send![&*blit,
                copyFromTexture: &*source, sourceSlice: 0usize, sourceLevel: 0usize,
                sourceOrigin: origin, sourceSize: size,
                toTexture: &*destination, destinationSlice: 0usize, destinationLevel: 0usize,
                destinationOrigin: origin];
            let _: () = msg_send![&*blit, endEncoding];
            let _: () = msg_send![&*command, commit];
            // No timeout or error return can release the borrowed frame before
            // the command reaches a terminal state. Keep both textures alive.
            let _: () = msg_send![&*command, waitUntilCompleted];
            let status: usize = msg_send![&*command, status];
            if status != 4 {
                return Err(format!(
                    "Metal capture completed with command status {status}"
                ));
            }
        }
        Ok(())
    }

    pub fn prepare_publication(&self) -> Result<(), String> {
        // Capture has completed on the very queue that will consume staging.
        // Tracked-resource dependencies provide Metal visibility; no external
        // queue handoff or texture layout transition is involved.
        Ok(())
    }
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
    let rd = RenderingServer::singleton().get_rendering_device()?;
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
