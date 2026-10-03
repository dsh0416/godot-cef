use ash::vk;
use ash::vk::Handle;
use godot::classes::RenderingServer;
use godot::classes::rendering_device::DriverResource;
use godot::global::godot_warn;
use godot::prelude::*;
use std::collections::VecDeque;
use std::time::Duration;

use super::super::{NativeCaptureTarget, SnapshotFormat};
use crate::accelerated_osr::vulkan_common::{
    VulkanCopyContext, find_memory_type_index, get_godot_gpu_device_ids_vulkan,
    submit_vulkan_copy_async,
};

const MAX_PENDING_PUBLICATION_BRIDGES: usize = 8;

/// Capture uses a reserved queue in Godot's graphics family when available.
/// Queue interception serializes host access, including Godot's transfer workers.
/// All GPU reads from CEF finish in `capture`, before its paint callback returns.
pub struct VulkanTextureImporter {
    device: ash::Device,
    instance: ash::Instance,
    physical_device: vk::PhysicalDevice,
    external_memory: ash::khr::external_memory_win32::Device,
    command_pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,
    fence: vk::Fence,
    queue: vk::Queue,
    godot_queue: vk::Queue,
    queue_family_index: u32,
    publication_bridges: VecDeque<PublicationBridge>,
    captured: bool,
    // Own the loader, not Godot's VkInstance/VkDevice. Function tables must not
    // outlive the DLL from which their entry points were resolved.
    _entry: ash::Entry,
}

impl VulkanTextureImporter {
    /// Called on the render thread while querying Godot's native handles.
    pub fn new() -> Result<Self, String> {
        let rd = RenderingServer::singleton()
            .get_rendering_device()
            .ok_or("Failed to get RenderingDevice")?;
        let device_handle = rd.get_driver_resource(DriverResource::LOGICAL_DEVICE, Rid::Invalid, 0);
        let instance_handle =
            rd.get_driver_resource(DriverResource::TOPMOST_OBJECT, Rid::Invalid, 0);
        let physical_handle =
            rd.get_driver_resource(DriverResource::PHYSICAL_DEVICE, Rid::Invalid, 0);
        let queue_handle = rd.get_driver_resource(DriverResource::COMMAND_QUEUE, Rid::Invalid, 0);
        let queue_family_index =
            rd.get_driver_resource(DriverResource::QUEUE_FAMILY, Rid::Invalid, 0) as u32;
        if [
            device_handle,
            instance_handle,
            physical_handle,
            queue_handle,
        ]
        .contains(&0)
        {
            return Err(
                "Godot did not expose its Vulkan device, instance, physical device, or queue"
                    .into(),
            );
        }
        let entry = unsafe { ash::Entry::load_from("vulkan-1.dll") }
            .map_err(|e| format!("Load Vulkan entry points failed: {e}"))?;
        let instance = unsafe {
            ash::Instance::load(entry.static_fn(), vk::Instance::from_raw(instance_handle))
        };
        let device =
            unsafe { ash::Device::load(instance.fp_v1_0(), vk::Device::from_raw(device_handle)) };
        let godot_queue = vk::Queue::from_raw(queue_handle);
        let queue = crate::vulkan_hook::queue_sync::choose_capture_queue(
            device.handle(),
            godot_queue,
            queue_family_index,
        )?;
        if unsafe {
            instance.get_device_proc_addr(
                device.handle(),
                c"vkGetMemoryWin32HandlePropertiesKHR".as_ptr(),
            )
        }
        .is_none()
        {
            return Err("VK_KHR_external_memory_win32 is not enabled on Godot's device".into());
        }
        let external_memory = ash::khr::external_memory_win32::Device::new(&instance, &device);
        let physical_device = vk::PhysicalDevice::from_raw(physical_handle);
        let pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(queue_family_index)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        let command_pool = unsafe { device.create_command_pool(&pool_info, None) }
            .map_err(|e| format!("Create capture command pool failed: {e:?}"))?;
        let allocation = vk::CommandBufferAllocateInfo::default()
            .command_pool(command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let command_buffer = match unsafe { device.allocate_command_buffers(&allocation) } {
            Ok(buffers) => buffers[0],
            Err(error) => {
                unsafe { device.destroy_command_pool(command_pool, None) };
                return Err(format!("Allocate capture command buffer failed: {error:?}"));
            }
        };
        let fence_info = vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);
        let fence = match unsafe { device.create_fence(&fence_info, None) } {
            Ok(fence) => fence,
            Err(error) => {
                unsafe { device.destroy_command_pool(command_pool, None) };
                return Err(format!("Create capture fence failed: {error:?}"));
            }
        };
        Ok(Self {
            device,
            instance,
            physical_device,
            external_memory,
            command_pool,
            command_buffer,
            fence,
            queue,
            godot_queue,
            queue_family_index,
            publication_bridges: VecDeque::with_capacity(MAX_PENDING_PUBLICATION_BRIDGES),
            captured: false,
            _entry: entry,
        })
    }

    pub fn capture(
        &mut self,
        info: &cef::AcceleratedPaintInfo,
        target: NativeCaptureTarget,
    ) -> Result<(), String> {
        if info.shared_texture_handle.is_null()
            || info.shared_texture_handle as isize == -1
            || info.extra.coded_size.width <= 0
            || info.extra.coded_size.height <= 0
            || target.native_handle == 0
            || target.width != info.extra.coded_size.width as u32
            || target.height != info.extra.coded_size.height as u32
        {
            return Err("Invalid CEF handle or mismatched Vulkan staging dimensions".into());
        }
        let expected_color = match target.format {
            SnapshotFormat::Bgra8 => cef::ColorType::BGRA_8888,
            SnapshotFormat::Rgba8 => cef::ColorType::RGBA_8888,
        };
        if info.format != expected_color {
            return Err("CEF pixel format does not match the Vulkan staging slot".into());
        }
        // Reclaim only after the consumer wait completed, independently of CEF
        // source completion. At most eight pending waits can hold GPU objects.
        self.reclaim_publication_bridges()?;
        let source = self.import_source(info.shared_texture_handle as isize, target)?;
        let bridge = if self.queue != self.godot_queue {
            Some(self.create_publication_bridge()?)
        } else {
            None
        };
        let functions = self.device.fp_v1_0();
        let context = VulkanCopyContext {
            device: self.device.handle(),
            queue: self.queue,
            queue_family_index: self.queue_family_index,
            src_external_queue_family: vk::QUEUE_FAMILY_EXTERNAL,
            wait_semaphore: vk::Semaphore::null(),
            signal_semaphore: bridge
                .as_ref()
                .map_or(vk::Semaphore::null(), |bridge| bridge.semaphore),
            reset_fences: functions.reset_fences,
            reset_command_buffer: functions.reset_command_buffer,
            begin_command_buffer: functions.begin_command_buffer,
            end_command_buffer: functions.end_command_buffer,
            cmd_pipeline_barrier: functions.cmd_pipeline_barrier,
            cmd_copy_image: functions.cmd_copy_image,
            queue_submit: functions.queue_submit,
        };
        // CEF 154.0.32 requests kPreferMappableSharedImage and holds the frame
        // until this callback returns. Its Chromium 154.0.8037.58 blit marks the
        // target mappable, so CopyOutputRGBA delivers it after GPU completion:
        // https://github.com/chromiumembedded/cef/blob/682c378d70d5780061e96644dca16ddd8fd157a9/libcef/browser/osr/video_consumer_osr.cc
        // https://github.com/chromium/chromium/blob/154.0.8037.58/components/viz/service/frame_sinks/video_capture/frame_sink_video_capturer_impl.cc
        // https://github.com/chromium/chromium/blob/154.0.8037.58/components/viz/service/display_embedder/skia_output_surface_impl_on_gpu.cc
        // Producer completion precedes capture; Vulkan's external-image rules
        // specify GENERAL for D3D11_TEXTURE imports. Acquire/release transfers
        // external ownership and restores that layout before CEF can reuse it:
        // https://github.com/KhronosGroup/Vulkan-Docs/blob/ab80b9e8dd1c08b14c9536c37a31182114ae72ee/chapters/resources.adoc#L5669-L5686
        if let Err(error) = submit_vulkan_copy_async(
            &context,
            self.command_buffer,
            self.fence,
            source.image,
            vk::Image::from_raw(target.native_handle),
            target.width,
            target.height,
        ) {
            // A submission error must not shorten CEF's borrow if the driver
            // accepted any work before reporting failure. Drain the actual
            // queue (or confirm device loss) while the imported source is live.
            self.finish_failed_submission(self.queue);
            if let Some(bridge) = bridge {
                self.destroy_publication_bridge(bridge);
            }
            return Err(error);
        }
        if let Err(error) = self.wait_for_capture() {
            // This wait returns early only on actual device loss. Neither the
            // borrowed image nor its signal semaphore can still be executing.
            if let Some(bridge) = bridge {
                self.destroy_publication_bridge(bridge);
            }
            return Err(error);
        }
        // source's RAII destructor runs after all copy/release commands finish.
        drop(source);
        if let Some(bridge) = bridge {
            self.enqueue_publication_bridge(bridge)?;
        }
        self.captured = true;
        Ok(())
    }

    pub fn prepare_publication(&self) -> Result<(), String> {
        if !self.captured {
            return Err("No completed Vulkan snapshot to publish".into());
        }
        // Capture restored TRANSFER_SRC_OPTIMAL. On the same queue, its final
        // barrier covers later RD copies. On a reserved queue, capture also
        // enqueued a semaphore wait on Godot's actual queue before publishing
        // the slot as Ready, so subsequent RD transfer reads inherit visibility.
        Ok(())
    }

    fn create_publication_bridge(&self) -> Result<PublicationBridge, String> {
        let semaphore = unsafe {
            self.device
                .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
        }
        .map_err(|error| format!("Create publication semaphore failed: {error:?}"))?;
        let fence = match unsafe {
            self.device
                .create_fence(&vk::FenceCreateInfo::default(), None)
        } {
            Ok(fence) => fence,
            Err(error) => {
                unsafe { self.device.destroy_semaphore(semaphore, None) };
                return Err(format!("Create publication wait fence failed: {error:?}"));
            }
        };
        Ok(PublicationBridge { semaphore, fence })
    }

    fn enqueue_publication_bridge(&mut self, bridge: PublicationBridge) -> Result<(), String> {
        let semaphores = [bridge.semaphore];
        let stages = [vk::PipelineStageFlags::TRANSFER];
        let submit = vk::SubmitInfo::default()
            .wait_semaphores(&semaphores)
            .wait_dst_stage_mask(&stages);
        // A semaphore wait's second scope includes later submissions to this
        // queue, so an empty batch orders the RD copies recorded after capture.
        // The semaphore supplies cross-queue write visibility; CPU fence waiting
        // alone would not. Both queues belong to the same graphics family, so
        // the EXCLUSIVE staging image requires no queue-family ownership transfer.
        // https://docs.vulkan.org/refpages/latest/refpages/source/vkQueueSubmit.html
        if let Err(error) = unsafe {
            self.device
                .queue_submit(self.godot_queue, &[submit], bridge.fence)
        } {
            // Even an ambiguous failed submit may have accepted the wait. The
            // private queue already completed, but only draining Godot's queue
            // (or device loss) permits destroying this possibly pending semaphore.
            self.finish_failed_submission(self.godot_queue);
            self.destroy_publication_bridge(bridge);
            return Err(format!("Submit Vulkan publication wait failed: {error:?}"));
        }
        self.publication_bridges.push_back(bridge);
        Ok(())
    }

    fn reclaim_publication_bridges(&mut self) -> Result<(), String> {
        while let Some(bridge) = self.publication_bridges.front() {
            match unsafe { self.device.get_fence_status(bridge.fence) } {
                Ok(true) => {}
                Ok(false) if self.publication_bridges.len() < MAX_PENDING_PUBLICATION_BRIDGES => {
                    return Ok(());
                }
                Ok(false) => {
                    // Actual consumer backpressure is the only ordinary-path
                    // reason to wait for Godot. Do so before creating a ninth pair.
                    self.wait_for_fence(bridge.fence, "publication wait reclamation")?;
                }
                Err(error) => {
                    // Keep every pending pair owned. Teardown waits for their
                    // fences (or confirmed device loss), never just a frame count.
                    return Err(format!("Query publication wait fence failed: {error:?}"));
                }
            }
            if let Some(bridge) = self.publication_bridges.pop_front() {
                self.destroy_publication_bridge(bridge);
            }
        }
        Ok(())
    }

    fn destroy_publication_bridge(&self, bridge: PublicationBridge) {
        // Caller proved that all submitted uses completed or the device was lost.
        unsafe {
            self.device.destroy_fence(bridge.fence, None);
            self.device.destroy_semaphore(bridge.semaphore, None);
        }
    }

    fn finish_failed_submission(&self, queue: vk::Queue) {
        let mut warned = false;
        loop {
            match unsafe { self.device.queue_wait_idle(queue) } {
                Ok(()) | Err(vk::Result::ERROR_DEVICE_LOST) => return,
                Err(error) => {
                    if !warned {
                        godot_warn!(
                            "[AcceleratedOSR/Vulkan] Queue drain failed ({error:?}); retaining GPU resources until completion or device loss"
                        );
                        warned = true;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
    }

    fn import_source(
        &self,
        handle: isize,
        target: NativeCaptureTarget,
    ) -> Result<ImportedVulkanImage, String> {
        let format = match target.format {
            SnapshotFormat::Bgra8 => vk::Format::B8G8R8A8_UNORM,
            SnapshotFormat::Rgba8 => vk::Format::R8G8B8A8_UNORM,
        };
        self.validate_import_format(format, target)?;
        let mut external_info = vk::ExternalMemoryImageCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::D3D11_TEXTURE);
        let image_info = vk::ImageCreateInfo::default()
            .push_next(&mut external_info)
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(vk::Extent3D {
                width: target.width,
                height: target.height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let image = unsafe { self.device.create_image(&image_info, None) }
            .map_err(|e| format!("Create imported image failed: {e:?}"))?;
        let mut imported = ImportedVulkanImage {
            device: self.device.clone(),
            image,
            memory: vk::DeviceMemory::null(),
        };
        let requirements = unsafe { self.device.get_image_memory_requirements(image) };
        let mut handle_properties = vk::MemoryWin32HandlePropertiesKHR::default();
        unsafe {
            self.external_memory.get_memory_win32_handle_properties(
                vk::ExternalMemoryHandleTypeFlags::D3D11_TEXTURE,
                handle,
                &mut handle_properties,
            )
        }
        .map_err(|e| format!("Query CEF handle memory properties failed: {e:?}"))?;
        let memory_type_index = find_memory_type_index(
            requirements.memory_type_bits & handle_properties.memory_type_bits,
        )
        .ok_or("CEF texture has no compatible Vulkan image memory type")?;
        let mut import_info = vk::ImportMemoryWin32HandleInfoKHR::default()
            .handle_type(vk::ExternalMemoryHandleTypeFlags::D3D11_TEXTURE)
            .handle(handle);
        let mut dedicated_info = vk::MemoryDedicatedAllocateInfo::default().image(image);
        let allocation = vk::MemoryAllocateInfo::default()
            .push_next(&mut import_info)
            .push_next(&mut dedicated_info)
            .allocation_size(requirements.size)
            .memory_type_index(memory_type_index);
        imported.memory = unsafe { self.device.allocate_memory(&allocation, None) }
            .map_err(|e| format!("Import CEF texture memory failed: {e:?}"))?;
        unsafe { self.device.bind_image_memory(image, imported.memory, 0) }
            .map_err(|e| format!("Bind imported image memory failed: {e:?}"))?;
        // Win32 NT-handle import retains the memory payload, but never takes
        // ownership of CEF's borrowed HANDLE. It must not be closed here.
        Ok(imported)
    }

    fn validate_import_format(
        &self,
        format: vk::Format,
        target: NativeCaptureTarget,
    ) -> Result<(), String> {
        let mut external_info = vk::PhysicalDeviceExternalImageFormatInfo::default()
            .handle_type(vk::ExternalMemoryHandleTypeFlags::D3D11_TEXTURE);
        let format_info = vk::PhysicalDeviceImageFormatInfo2::default()
            .push_next(&mut external_info)
            .format(format)
            .ty(vk::ImageType::TYPE_2D)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::TRANSFER_SRC);
        let mut external_properties = vk::ExternalImageFormatProperties::default();
        let mut format_properties =
            vk::ImageFormatProperties2::default().push_next(&mut external_properties);
        unsafe {
            self.instance.get_physical_device_image_format_properties2(
                self.physical_device,
                &format_info,
                &mut format_properties,
            )
        }
        .map_err(|e| format!("Query D3D11 texture import support failed: {e:?}"))?;
        let max_extent = format_properties.image_format_properties.max_extent;
        if !external_properties
            .external_memory_properties
            .external_memory_features
            .contains(vk::ExternalMemoryFeatureFlags::IMPORTABLE)
            || target.width > max_extent.width
            || target.height > max_extent.height
        {
            return Err("The Vulkan device cannot import this D3D11 texture format/extent".into());
        }
        Ok(())
    }

    fn wait_for_capture(&self) -> Result<(), String> {
        self.wait_for_fence(self.fence, "CEF capture")
    }

    fn wait_for_fence(&self, fence: vk::Fence, operation: &str) -> Result<(), String> {
        let mut wait_error_logged = false;
        loop {
            match unsafe { self.device.wait_for_fences(&[fence], true, 100_000_000) } {
                Ok(()) => return Ok(()),
                Err(vk::Result::ERROR_DEVICE_LOST) => {
                    return Err(format!("Vulkan device lost during {operation}"));
                }
                Err(vk::Result::TIMEOUT) => {}
                Err(error) => {
                    // Returning on OOM/other host errors would release a source
                    // that the GPU may still read. Preserve the borrow and retry.
                    if !wait_error_logged {
                        godot_warn!(
                            "[AcceleratedOSR/Vulkan] Fence wait for {operation} failed ({error:?}); retaining GPU resources until completion or device loss"
                        );
                        wait_error_logged = true;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
    }
}

// These handles have no unconditional Drop: every release must be backed by a
// completed consumer fence, an error-path queue drain, or confirmed device loss.
struct PublicationBridge {
    semaphore: vk::Semaphore,
    fence: vk::Fence,
}

struct ImportedVulkanImage {
    device: ash::Device,
    image: vk::Image,
    memory: vk::DeviceMemory,
}

impl Drop for ImportedVulkanImage {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_image(self.image, None);
            if self.memory != vk::DeviceMemory::null() {
                self.device.free_memory(self.memory, None);
            }
        }
    }
}

impl Drop for VulkanTextureImporter {
    fn drop(&mut self) {
        // Every submitted capture was waited to completion (or device loss).
        // Consumer waits can outlive capture, so drain them while Godot's device
        // is still alive before releasing their semaphore/fence pairs.
        while let Some(bridge) = self.publication_bridges.pop_front() {
            let _ = self.wait_for_fence(bridge.fence, "publication teardown");
            self.destroy_publication_bridge(bridge);
        }
        unsafe {
            self.device.destroy_fence(self.fence, None);
            self.device.destroy_command_pool(self.command_pool, None);
        }
    }
}

pub fn get_godot_gpu_device_ids() -> Option<(u32, u32)> {
    get_godot_gpu_device_ids_vulkan("vulkan-1.dll")
}
