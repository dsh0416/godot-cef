//! Linux Vulkan texture importer using DMA-BUF external memory.
//!
//! This module imports DMA-BUF file descriptors from CEF into Vulkan images
//! and copies them to Godot's RenderingDevice textures.

use ash::vk;
use cef::ColorType;
use godot::classes::RenderingServer;
use godot::classes::rendering_device::DriverResource;
use godot::global::{godot_error, godot_print};
use godot::prelude::*;
use std::os::fd::RawFd;

/// DRM format modifier indicating invalid/linear modifier
const DRM_FORMAT_MOD_INVALID: u64 = 0x00ffffffffffffff;

/// Parameters for DMA-BUF import extracted from AcceleratedPaintInfo
struct DmaBufImportParams {
    /// File descriptors for each plane
    fds: Vec<RawFd>,
    /// Stride (pitch) for each plane
    strides: Vec<u32>,
    /// Offset for each plane
    offsets: Vec<u64>,
    /// DRM format modifier
    modifier: u64,
    /// Vulkan format to use
    format: vk::Format,
    /// Image dimensions
    width: u32,
    height: u32,
}

type PfnVkGetMemoryFdPropertiesKHR = unsafe extern "system" fn(
    device: vk::Device,
    handle_type: vk::ExternalMemoryHandleTypeFlags,
    fd: RawFd,
    p_memory_fd_properties: *mut vk::MemoryFdPropertiesKHR<'_>,
) -> vk::Result;

const FRAME_BUFFER_COUNT: usize = 2;

pub struct VulkanTextureImporter {
    device: vk::Device,
    command_pool: vk::CommandPool,
    command_buffers: [vk::CommandBuffer; FRAME_BUFFER_COUNT],
    fences: [vk::Fence; FRAME_BUFFER_COUNT],
    queue: vk::Queue,
    current_frame: usize,
    get_memory_fd_properties: PfnVkGetMemoryFdPropertiesKHR,
    cached_memory_type_index: Option<u32>,
    imported_image: Option<ImportedVulkanImage>,
}

struct ImportedVulkanImage {
    fd_value: RawFd,
    image: vk::Image,
    memory: vk::DeviceMemory,
    extent: vk::Extent2D,
}

struct VulkanFunctions {
    destroy_image: vk::PFN_vkDestroyImage,
    free_memory: vk::PFN_vkFreeMemory,
    allocate_memory: vk::PFN_vkAllocateMemory,
    bind_image_memory: vk::PFN_vkBindImageMemory,
    create_image: vk::PFN_vkCreateImage,
    create_command_pool: vk::PFN_vkCreateCommandPool,
    destroy_command_pool: vk::PFN_vkDestroyCommandPool,
    allocate_command_buffers: vk::PFN_vkAllocateCommandBuffers,
    create_fence: vk::PFN_vkCreateFence,
    destroy_fence: vk::PFN_vkDestroyFence,
    begin_command_buffer: vk::PFN_vkBeginCommandBuffer,
    end_command_buffer: vk::PFN_vkEndCommandBuffer,
    cmd_pipeline_barrier: vk::PFN_vkCmdPipelineBarrier,
    cmd_copy_image: vk::PFN_vkCmdCopyImage,
    queue_submit: vk::PFN_vkQueueSubmit,
    wait_for_fences: vk::PFN_vkWaitForFences,
    reset_fences: vk::PFN_vkResetFences,
    reset_command_buffer: vk::PFN_vkResetCommandBuffer,
    get_device_queue: vk::PFN_vkGetDeviceQueue,
    get_memory_fd_properties: PfnVkGetMemoryFdPropertiesKHR,
}

static VULKAN_FNS: std::sync::OnceLock<VulkanFunctions> = std::sync::OnceLock::new();

impl VulkanTextureImporter {
    pub fn new() -> Option<Self> {
        let mut rd = RenderingServer::singleton()
            .get_rendering_device()
            .ok_or_else(|| {
                godot_error!("[AcceleratedOSR/Vulkan] Failed to get RenderingDevice");
            })
            .ok()?;

        // Get the Vulkan device from Godot (cast directly to vk::Device which is just a u64 handle)
        let device_ptr = rd.get_driver_resource(DriverResource::LOGICAL_DEVICE, Rid::Invalid, 0);
        if device_ptr == 0 {
            godot_error!("[AcceleratedOSR/Vulkan] Failed to get Vulkan device from Godot");
            return None;
        }
        let device: vk::Device = unsafe { std::mem::transmute(device_ptr) };

        // Load Vulkan library and function pointers
        let lib = match unsafe { libloading::Library::new("libvulkan.so.1") } {
            Ok(lib) => lib,
            Err(e) => {
                godot_error!(
                    "[AcceleratedOSR/Vulkan] Failed to load libvulkan.so.1: {}",
                    e
                );
                return None;
            }
        };

        // Load function pointers using the device
        let fns = VULKAN_FNS.get_or_init(|| Self::load_vulkan_functions(&lib, device));

        // We need to find the physical device. Use the queue to infer it's valid.
        // Godot uses queue family 0 for graphics by default.
        let queue_family_index = 0u32;
        let mut queue: vk::Queue = unsafe { std::mem::zeroed() };
        unsafe {
            (fns.get_device_queue)(device, queue_family_index, 0, &mut queue);
        }

        if queue == vk::Queue::null() {
            godot_error!("[AcceleratedOSR/Vulkan] Failed to get graphics queue");
            return None;
        }

        // Create command pool
        let pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(queue_family_index)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);

        let mut command_pool: vk::CommandPool = unsafe { std::mem::zeroed() };
        let result = unsafe {
            (fns.create_command_pool)(device, &pool_info, std::ptr::null(), &mut command_pool)
        };
        if result != vk::Result::SUCCESS {
            godot_error!(
                "[AcceleratedOSR/Vulkan] Failed to create command pool: {:?}",
                result
            );
            return None;
        }

        // Allocate double-buffered command buffers
        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(FRAME_BUFFER_COUNT as u32);

        let mut command_buffers: [vk::CommandBuffer; FRAME_BUFFER_COUNT] =
            unsafe { std::mem::zeroed() };
        let result = unsafe {
            (fns.allocate_command_buffers)(device, &alloc_info, command_buffers.as_mut_ptr())
        };
        if result != vk::Result::SUCCESS {
            godot_error!(
                "[AcceleratedOSR/Vulkan] Failed to allocate command buffers: {:?}",
                result
            );
            unsafe {
                (fns.destroy_command_pool)(device, command_pool, std::ptr::null());
            }
            return None;
        }

        // Create double-buffered fences (start signaled so first wait doesn't block)
        let fence_info = vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);
        let mut fences: [vk::Fence; FRAME_BUFFER_COUNT] = unsafe { std::mem::zeroed() };
        for i in 0..FRAME_BUFFER_COUNT {
            let result = unsafe {
                (fns.create_fence)(device, &fence_info, std::ptr::null(), &mut fences[i])
            };
            if result != vk::Result::SUCCESS {
                godot_error!(
                    "[AcceleratedOSR/Vulkan] Failed to create fence: {:?}",
                    result
                );
                for fence in fences.iter().take(i) {
                    unsafe {
                        (fns.destroy_fence)(device, *fence, std::ptr::null());
                    }
                }
                unsafe {
                    (fns.destroy_command_pool)(device, command_pool, std::ptr::null());
                }
                return None;
            }
        }

        // Keep library loaded for the lifetime of the importer
        std::mem::forget(lib);

        godot_print!(
            "[AcceleratedOSR/Vulkan] Using Godot's Vulkan device for accelerated OSR (double-buffered)"
        );

        Some(Self {
            device,
            command_pool,
            command_buffers,
            queue,
            fences,
            current_frame: 0,
            get_memory_fd_properties: fns.get_memory_fd_properties,
            cached_memory_type_index: None,
            imported_image: None,
        })
    }

    fn load_vulkan_functions(lib: &libloading::Library, device: vk::Device) -> VulkanFunctions {
        type GetDeviceProcAddr = unsafe extern "system" fn(
            vk::Device,
            *const std::ffi::c_char,
        ) -> vk::PFN_vkVoidFunction;

        let get_device_proc_addr: GetDeviceProcAddr = unsafe {
            *lib.get(b"vkGetDeviceProcAddr\0")
                .expect("Failed to get vkGetDeviceProcAddr")
        };

        // Macro to load device functions
        macro_rules! load_device_fn {
            ($fn_name:expr, $fn_type:ty) => {
                unsafe {
                    let ptr =
                        get_device_proc_addr(device, concat!($fn_name, "\0").as_ptr() as *const _);
                    if ptr.is_none() {
                        panic!("Failed to load Vulkan function: {}", $fn_name);
                    }
                    std::mem::transmute::<vk::PFN_vkVoidFunction, $fn_type>(ptr)
                }
            };
        }

        VulkanFunctions {
            destroy_image: load_device_fn!("vkDestroyImage", vk::PFN_vkDestroyImage),
            free_memory: load_device_fn!("vkFreeMemory", vk::PFN_vkFreeMemory),
            allocate_memory: load_device_fn!("vkAllocateMemory", vk::PFN_vkAllocateMemory),
            bind_image_memory: load_device_fn!("vkBindImageMemory", vk::PFN_vkBindImageMemory),
            create_image: load_device_fn!("vkCreateImage", vk::PFN_vkCreateImage),
            create_command_pool: load_device_fn!(
                "vkCreateCommandPool",
                vk::PFN_vkCreateCommandPool
            ),
            destroy_command_pool: load_device_fn!(
                "vkDestroyCommandPool",
                vk::PFN_vkDestroyCommandPool
            ),
            allocate_command_buffers: load_device_fn!(
                "vkAllocateCommandBuffers",
                vk::PFN_vkAllocateCommandBuffers
            ),
            create_fence: load_device_fn!("vkCreateFence", vk::PFN_vkCreateFence),
            destroy_fence: load_device_fn!("vkDestroyFence", vk::PFN_vkDestroyFence),
            begin_command_buffer: load_device_fn!(
                "vkBeginCommandBuffer",
                vk::PFN_vkBeginCommandBuffer
            ),
            end_command_buffer: load_device_fn!("vkEndCommandBuffer", vk::PFN_vkEndCommandBuffer),
            cmd_pipeline_barrier: load_device_fn!(
                "vkCmdPipelineBarrier",
                vk::PFN_vkCmdPipelineBarrier
            ),
            cmd_copy_image: load_device_fn!("vkCmdCopyImage", vk::PFN_vkCmdCopyImage),
            queue_submit: load_device_fn!("vkQueueSubmit", vk::PFN_vkQueueSubmit),
            wait_for_fences: load_device_fn!("vkWaitForFences", vk::PFN_vkWaitForFences),
            reset_fences: load_device_fn!("vkResetFences", vk::PFN_vkResetFences),
            reset_command_buffer: load_device_fn!(
                "vkResetCommandBuffer",
                vk::PFN_vkResetCommandBuffer
            ),
            get_device_queue: load_device_fn!("vkGetDeviceQueue", vk::PFN_vkGetDeviceQueue),
            get_memory_fd_properties: load_device_fn!(
                "vkGetMemoryFdPropertiesKHR",
                PfnVkGetMemoryFdPropertiesKHR
            ),
        }
    }

    pub fn import_and_copy(
        &mut self,
        info: &cef::AcceleratedPaintInfo,
        dst_rd_rid: Rid,
    ) -> Result<(), String> {
        let fns = VULKAN_FNS.get().ok_or("Vulkan functions not loaded")?;

        // Get current frame index and wait for its previous use to complete
        let frame_idx = self.current_frame;
        let fence = self.fences[frame_idx];

        // Wait for THIS frame's previous use to complete (allows other frame to be in-flight)
        let _ = unsafe { (fns.wait_for_fences)(self.device, 1, &fence, vk::TRUE, u64::MAX) };

        // Extract DMA-BUF parameters from all planes
        let plane_count = info.plane_count as usize;
        if plane_count == 0 {
            return Err("No planes in AcceleratedPaintInfo".into());
        }

        let mut fds = Vec::with_capacity(plane_count);
        let mut strides = Vec::with_capacity(plane_count);
        let mut offsets = Vec::with_capacity(plane_count);

        for i in 0..plane_count {
            let plane = info
                .planes
                .get(i)
                .ok_or_else(|| format!("Missing plane {} (plane_count={})", i, plane_count))?;
            if plane.fd < 0 {
                return Err(format!("Invalid fd for plane {}: {}", i, plane.fd));
            }
            fds.push(plane.fd);
            strides.push(plane.stride);
            offsets.push(plane.offset);
        }

        let width = info.extra.coded_size.width as u32;
        let height = info.extra.coded_size.height as u32;

        if width == 0 || height == 0 {
            return Err(format!("Invalid source dimensions: {}x{}", width, height));
        }
        if !dst_rd_rid.is_valid() {
            return Err("Destination RID is invalid".into());
        }

        // Convert CEF color format to Vulkan format
        // Note: CEF format names are reversed from DRM perspective
        // CEF_COLOR_TYPE_RGBA_8888 -> DRM_FORMAT_ABGR8888 -> VK_FORMAT_R8G8B8A8
        // CEF_COLOR_TYPE_BGRA_8888 -> DRM_FORMAT_ARGB8888 -> VK_FORMAT_B8G8R8A8
        let format = cef_format_to_vulkan(&info.format);

        let params = DmaBufImportParams {
            fds,
            strides,
            offsets,
            modifier: info.modifier,
            format,
            width,
            height,
        };

        // Import the DMA-BUF as a Vulkan image
        let src_image = self.import_dmabuf_to_image(&params)?;

        // Get destination Vulkan image from Godot's RenderingDevice
        let dst_image: vk::Image = {
            let mut rd = RenderingServer::singleton()
                .get_rendering_device()
                .ok_or("Failed to get RenderingDevice")?;

            let image_ptr = rd.get_driver_resource(DriverResource::TEXTURE, dst_rd_rid, 0);
            if image_ptr == 0 {
                return Err("Failed to get destination Vulkan image".into());
            }

            unsafe { std::mem::transmute(image_ptr) }
        };

        // Copy from imported image to Godot's texture
        self.submit_copy(src_image, dst_image, width, height, frame_idx)?;

        // Advance to next frame for double buffering
        self.current_frame = (self.current_frame + 1) % FRAME_BUFFER_COUNT;

        Ok(())
    }

    fn import_dmabuf_to_image(&mut self, params: &DmaBufImportParams) -> Result<vk::Image, String> {
        let fns = VULKAN_FNS.get().ok_or("Vulkan functions not loaded")?;
        let extent = vk::Extent2D {
            width: params.width,
            height: params.height,
        };

        // Check if we can fully reuse existing import (same primary fd AND dimensions)
        let primary_fd = params.fds[0];
        if let Some(existing) = &self.imported_image
            && existing.fd_value == primary_fd
            && existing.extent == extent
        {
            // Cache hit! Reuse everything
            return Ok(existing.image);
        }

        // Cache miss - must create new image (VkImage can only be bound once)
        self.free_imported_image();

        // Create new image with external memory flag for DMA-BUF
        let mut external_memory_info = vk::ExternalMemoryImageCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);

        // Build plane layouts for DRM format modifier
        let plane_layouts: Vec<vk::SubresourceLayout> = params
            .fds
            .iter()
            .enumerate()
            .map(|(i, _)| vk::SubresourceLayout {
                offset: params.offsets.get(i).copied().unwrap_or(0),
                size: 0, // Calculated by driver
                row_pitch: params.strides.get(i).copied().unwrap_or(0) as u64,
                array_pitch: 0,
                depth_pitch: 0,
            })
            .collect();

        // Set up DRM format modifier info if we have a valid modifier
        let use_drm_modifier = params.modifier != DRM_FORMAT_MOD_INVALID;

        let mut drm_modifier_info = vk::ImageDrmFormatModifierExplicitCreateInfoEXT::default()
            .drm_format_modifier(params.modifier)
            .plane_layouts(&plane_layouts);

        let tiling = if use_drm_modifier {
            vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT
        } else {
            vk::ImageTiling::LINEAR
        };

        let mut image_info = vk::ImageCreateInfo::default()
            .push_next(&mut external_memory_info)
            .image_type(vk::ImageType::TYPE_2D)
            .format(params.format)
            .extent(vk::Extent3D {
                width: params.width,
                height: params.height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(tiling)
            .usage(vk::ImageUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);

        // Only add DRM modifier info if we're using DRM tiling
        if use_drm_modifier {
            image_info = image_info.push_next(&mut drm_modifier_info);
        }

        let mut image = vk::Image::null();
        let result =
            unsafe { (fns.create_image)(self.device, &image_info, std::ptr::null(), &mut image) };
        if result != vk::Result::SUCCESS {
            return Err(format!(
                "Failed to create image: {:?} (format={:?}, tiling={:?}, modifier=0x{:x})",
                result, params.format, tiling, params.modifier
            ));
        }

        // Import memory for this DMA-BUF
        let memory = self.import_memory_for_dmabuf(params, image)?;

        self.imported_image = Some(ImportedVulkanImage {
            fd_value: primary_fd,
            image,
            memory,
            extent,
        });
        Ok(image)
    }

    fn import_memory_for_dmabuf(
        &mut self,
        params: &DmaBufImportParams,
        image: vk::Image,
    ) -> Result<vk::DeviceMemory, String> {
        let fns = VULKAN_FNS.get().ok_or("Vulkan functions not loaded")?;

        // Use the first plane's fd for memory import
        let fd = params.fds[0];

        // Get or cache the memory type index (same for all DMA-BUF imports)
        let memory_type_index = if let Some(cached) = self.cached_memory_type_index {
            cached
        } else {
            // Query memory properties for this fd (only once)
            let mut fd_props = vk::MemoryFdPropertiesKHR::default();
            let result = unsafe {
                (self.get_memory_fd_properties)(
                    self.device,
                    vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT,
                    fd,
                    &mut fd_props,
                )
            };
            if result != vk::Result::SUCCESS {
                return Err(format!("Failed to get memory fd properties: {:?}", result));
            }

            let idx = Self::find_memory_type_index(fd_props.memory_type_bits)
                .ok_or("Failed to find suitable memory type")?;
            self.cached_memory_type_index = Some(idx);
            idx
        };

        // Import the memory with the DMA-BUF fd
        // Note: The fd ownership is transferred to Vulkan upon successful import
        let mut import_info = vk::ImportMemoryFdInfoKHR::default()
            .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT)
            .fd(fd);

        let mut dedicated_info = vk::MemoryDedicatedAllocateInfo::default().image(image);

        let allocation_size = (params.width as u64) * (params.height as u64) * 4;

        let alloc_info = vk::MemoryAllocateInfo::default()
            .push_next(&mut import_info)
            .push_next(&mut dedicated_info)
            .allocation_size(allocation_size)
            .memory_type_index(memory_type_index);

        let mut memory = vk::DeviceMemory::null();
        let result = unsafe {
            (fns.allocate_memory)(self.device, &alloc_info, std::ptr::null(), &mut memory)
        };
        if result != vk::Result::SUCCESS {
            return Err(format!("Failed to allocate/import memory: {:?}", result));
        }

        // Bind image to memory
        let result = unsafe { (fns.bind_image_memory)(self.device, image, memory, 0) };
        if result != vk::Result::SUCCESS {
            unsafe {
                (fns.free_memory)(self.device, memory, std::ptr::null());
            }
            return Err(format!("Failed to bind image memory: {:?}", result));
        }

        Ok(memory)
    }

    fn find_memory_type_index(type_filter: u32) -> Option<u32> {
        if type_filter == 0 {
            return None;
        }
        Some(type_filter.trailing_zeros())
    }

    fn submit_copy(
        &mut self,
        src: vk::Image,
        dst: vk::Image,
        width: u32,
        height: u32,
        frame_idx: usize,
    ) -> Result<(), String> {
        let fns = VULKAN_FNS.get().ok_or("Vulkan functions not loaded")?;

        let fence = self.fences[frame_idx];
        let cmd_buffer = self.command_buffers[frame_idx];

        // Reset fence and command buffer for this frame
        let _ = unsafe { (fns.reset_fences)(self.device, 1, &fence) };
        let _ =
            unsafe { (fns.reset_command_buffer)(cmd_buffer, vk::CommandBufferResetFlags::empty()) };

        // Begin command buffer
        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);

        let _ = unsafe { (fns.begin_command_buffer)(cmd_buffer, &begin_info) };

        // Combined barrier: transition both src and dst in one call
        // Source: UNDEFINED -> TRANSFER_SRC (external memory is ready from CEF)
        // Dest: UNDEFINED -> TRANSFER_DST
        let subresource_range = vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        };

        let barriers = [
            vk::ImageMemoryBarrier::default()
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(src)
                .subresource_range(subresource_range)
                .src_access_mask(vk::AccessFlags::empty())
                .dst_access_mask(vk::AccessFlags::TRANSFER_READ),
            vk::ImageMemoryBarrier::default()
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(dst)
                .subresource_range(subresource_range)
                .src_access_mask(vk::AccessFlags::empty())
                .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE),
        ];

        unsafe {
            (fns.cmd_pipeline_barrier)(
                cmd_buffer,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
                2,
                barriers.as_ptr(),
            );
        }

        // Copy image
        let region = vk::ImageCopy {
            src_subresource: vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            },
            src_offset: vk::Offset3D::default(),
            dst_subresource: vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            },
            dst_offset: vk::Offset3D::default(),
            extent: vk::Extent3D {
                width,
                height,
                depth: 1,
            },
        };

        unsafe {
            (fns.cmd_copy_image)(
                cmd_buffer,
                src,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                dst,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                1,
                &region,
            );
        }

        // Transition destination to SHADER_READ_ONLY for sampling
        let final_barrier = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(dst)
            .subresource_range(subresource_range)
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ);

        unsafe {
            (fns.cmd_pipeline_barrier)(
                cmd_buffer,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
                1,
                &final_barrier,
            );
        }

        let _ = unsafe { (fns.end_command_buffer)(cmd_buffer) };

        // Submit without waiting - we'll wait at the start of the next frame's use of this slot
        let submit_info =
            vk::SubmitInfo::default().command_buffers(std::slice::from_ref(&cmd_buffer));

        let _ = unsafe { (fns.queue_submit)(self.queue, 1, &submit_info, fence) };

        Ok(())
    }

    fn free_imported_image(&mut self) {
        if let Some(img) = self.imported_image.take()
            && let Some(fns) = VULKAN_FNS.get()
        {
            unsafe {
                (fns.destroy_image)(self.device, img.image, std::ptr::null());
                (fns.free_memory)(self.device, img.memory, std::ptr::null());
            }
        }
    }
}

impl Drop for VulkanTextureImporter {
    fn drop(&mut self) {
        // Wait for all in-flight copies to complete before cleanup
        if let Some(fns) = VULKAN_FNS.get() {
            let _ = unsafe {
                (fns.wait_for_fences)(
                    self.device,
                    FRAME_BUFFER_COUNT as u32,
                    self.fences.as_ptr(),
                    vk::TRUE,
                    u64::MAX,
                )
            };
        }

        self.free_imported_image();

        if let Some(fns) = VULKAN_FNS.get() {
            unsafe {
                for fence in &self.fences {
                    (fns.destroy_fence)(self.device, *fence, std::ptr::null());
                }
                (fns.destroy_command_pool)(self.device, self.command_pool, std::ptr::null());
            }
        }
        // Note: device is owned by Godot, don't destroy it
    }
}

unsafe impl Send for VulkanTextureImporter {}
unsafe impl Sync for VulkanTextureImporter {}

/// Convert CEF color format to Vulkan format.
///
/// Note: CEF format names are from CPU perspective (memory order),
/// while DRM/Vulkan formats specify channel order in the packed value.
/// CEF_COLOR_TYPE_RGBA_8888 means R is at lowest address -> maps to ABGR in DRM -> R8G8B8A8 in Vulkan
/// CEF_COLOR_TYPE_BGRA_8888 means B is at lowest address -> maps to ARGB in DRM -> B8G8R8A8 in Vulkan
fn cef_format_to_vulkan(format: &ColorType) -> vk::Format {
    match *format {
        ColorType::RGBA_8888 => vk::Format::R8G8B8A8_SRGB,
        ColorType::BGRA_8888 => vk::Format::B8G8R8A8_SRGB,
        // Default to BGRA which is most common
        _ => vk::Format::B8G8R8A8_SRGB,
    }
}

pub fn get_godot_device_uuid() -> Option<[u8; 16]> {
    let mut rd = RenderingServer::singleton().get_rendering_device()?;

    let physical_device_ptr =
        rd.get_driver_resource(DriverResource::PHYSICAL_DEVICE, Rid::Invalid, 0);
    if physical_device_ptr == 0 {
        godot_error!("[AcceleratedOSR/Vulkan] Failed to get Vulkan physical device for UUID query");
        return None;
    }
    let physical_device: vk::PhysicalDevice = unsafe { std::mem::transmute(physical_device_ptr) };

    let lib = match unsafe { libloading::Library::new("libvulkan.so.1") } {
        Ok(lib) => lib,
        Err(e) => {
            godot_error!(
                "[AcceleratedOSR/Vulkan] Failed to load libvulkan.so.1 for UUID query: {}",
                e
            );
            return None;
        }
    };

    type GetPhysicalDeviceProperties2 = unsafe extern "system" fn(
        physical_device: vk::PhysicalDevice,
        p_properties: *mut vk::PhysicalDeviceProperties2<'_>,
    );

    let get_physical_device_properties2: GetPhysicalDeviceProperties2 = unsafe {
        match lib.get(b"vkGetPhysicalDeviceProperties2\0") {
            Ok(f) => *f,
            Err(e) => {
                godot_error!(
                    "[AcceleratedOSR/Vulkan] Failed to get vkGetPhysicalDeviceProperties2: {}. \
                     Vulkan 1.1+ is required for UUID query.",
                    e
                );
                return None;
            }
        }
    };

    let mut id_props = vk::PhysicalDeviceIDProperties::default();
    let mut props2 = vk::PhysicalDeviceProperties2::default().push_next(&mut id_props);

    unsafe {
        get_physical_device_properties2(physical_device, &mut props2);
    }

    let uuid = id_props.device_uuid;
    godot_print!(
        "[AcceleratedOSR/Vulkan] Godot device UUID: {:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        uuid[0],
        uuid[1],
        uuid[2],
        uuid[3],
        uuid[4],
        uuid[5],
        uuid[6],
        uuid[7],
        uuid[8],
        uuid[9],
        uuid[10],
        uuid[11],
        uuid[12],
        uuid[13],
        uuid[14],
        uuid[15]
    );

    Some(uuid)
}
