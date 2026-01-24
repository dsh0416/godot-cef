use ash::vk;
use godot::classes::RenderingServer;
use godot::classes::rendering_device::DriverResource;
use godot::global::{godot_error, godot_print};
use godot::prelude::*;
use windows::Win32::Foundation::{CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE};
use windows::Win32::System::Threading::GetCurrentProcess;

type PfnVkGetMemoryWin32HandlePropertiesKHR = unsafe extern "system" fn(
    device: vk::Device,
    handle_type: vk::ExternalMemoryHandleTypeFlags,
    handle: HANDLE,
    p_memory_win32_handle_properties: *mut vk::MemoryWin32HandlePropertiesKHR<'_>,
) -> vk::Result;

pub struct PendingVulkanCopy {
    duplicated_handle: HANDLE,
    width: u32,
    height: u32,
}

impl Drop for PendingVulkanCopy {
    fn drop(&mut self) {
        if !self.duplicated_handle.is_invalid() {
            let _ = unsafe { CloseHandle(self.duplicated_handle) };
        }
    }
}

pub struct VulkanTextureImporter {
    device: vk::Device,
    command_pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,
    fence: vk::Fence,
    queue: vk::Queue,
    queue_family_index: u32,
    uses_separate_queue: bool,
    get_memory_win32_handle_properties: PfnVkGetMemoryWin32HandlePropertiesKHR,
    cached_memory_type_index: Option<u32>,
    imported_image: Option<ImportedVulkanImage>,
    pending_copy: Option<PendingVulkanCopy>,
    copy_in_flight: bool,
}

struct ImportedVulkanImage {
    duplicated_handle: HANDLE,
    image: vk::Image,
    memory: vk::DeviceMemory,
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
    get_memory_win32_handle_properties: PfnVkGetMemoryWin32HandlePropertiesKHR,
}

static VULKAN_FNS: std::sync::OnceLock<VulkanFunctions> = std::sync::OnceLock::new();

fn duplicate_win32_handle(handle: HANDLE) -> Result<HANDLE, String> {
    let mut duplicated = HANDLE::default();
    let current_process = unsafe { GetCurrentProcess() };
    unsafe {
        DuplicateHandle(
            current_process,
            handle,
            current_process,
            &mut duplicated,
            0,
            false,
            DUPLICATE_SAME_ACCESS,
        )
        .map_err(|e| format!("DuplicateHandle failed: {:?}", e))?;
    }
    Ok(duplicated)
}

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
        let lib = match unsafe { libloading::Library::new("vulkan-1.dll") } {
            Ok(lib) => lib,
            Err(e) => {
                godot_error!("[AcceleratedOSR/Vulkan] Failed to load vulkan-1.dll: {}", e);
                return None;
            }
        };

        // Load function pointers using the device
        let fns = VULKAN_FNS.get_or_init(|| Self::load_vulkan_functions(&lib, device));

        // Get physical device from Godot to query queue families
        let physical_device_ptr =
            rd.get_driver_resource(DriverResource::PHYSICAL_DEVICE, Rid::Invalid, 0);
        let physical_device: vk::PhysicalDevice = if physical_device_ptr != 0 {
            unsafe { std::mem::transmute::<u64, vk::PhysicalDevice>(physical_device_ptr) }
        } else {
            vk::PhysicalDevice::null()
        };

        // Try to find a separate queue for our copy operations
        // This avoids synchronization issues with Godot's main graphics queue
        let (queue_family_index, queue_index, uses_separate_queue) =
            Self::find_copy_queue(&lib, physical_device, fns);

        let mut queue: vk::Queue = unsafe { std::mem::zeroed() };
        unsafe {
            (fns.get_device_queue)(device, queue_family_index, queue_index, &mut queue);
        }

        if queue == vk::Queue::null() {
            // Fall back to queue 0 if our preferred queue isn't available
            godot_print!(
                "[AcceleratedOSR/Vulkan] Preferred queue not available, falling back to queue 0"
            );
            unsafe {
                (fns.get_device_queue)(device, 0, 0, &mut queue);
            }
        }

        if queue == vk::Queue::null() {
            godot_error!("[AcceleratedOSR/Vulkan] Failed to get any Vulkan queue");
            return None;
        }

        // Create command pool for our queue family
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

        // Allocate command buffer
        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);

        let mut command_buffer: vk::CommandBuffer = unsafe { std::mem::zeroed() };
        let result =
            unsafe { (fns.allocate_command_buffers)(device, &alloc_info, &mut command_buffer) };
        if result != vk::Result::SUCCESS {
            godot_error!(
                "[AcceleratedOSR/Vulkan] Failed to allocate command buffer: {:?}",
                result
            );
            unsafe {
                (fns.destroy_command_pool)(device, command_pool, std::ptr::null());
            }
            return None;
        }

        // Create fence (start signaled so first reset doesn't fail)
        let fence_info = vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);
        let mut fence: vk::Fence = unsafe { std::mem::zeroed() };
        let result =
            unsafe { (fns.create_fence)(device, &fence_info, std::ptr::null(), &mut fence) };
        if result != vk::Result::SUCCESS {
            godot_error!(
                "[AcceleratedOSR/Vulkan] Failed to create fence: {:?}",
                result
            );
            unsafe {
                (fns.destroy_command_pool)(device, command_pool, std::ptr::null());
            }
            return None;
        }

        // Keep library loaded for the lifetime of the importer
        std::mem::forget(lib);

        if uses_separate_queue {
            godot_print!(
                "[AcceleratedOSR/Vulkan] Using separate queue (family={}, index={}) for texture copies",
                queue_family_index,
                queue_index
            );
        } else {
            godot_print!(
                "[AcceleratedOSR/Vulkan] Using shared graphics queue - may have sync issues under load"
            );
        }

        Some(Self {
            device,
            command_pool,
            command_buffer,
            queue,
            queue_family_index,
            uses_separate_queue,
            fence,
            get_memory_win32_handle_properties: fns.get_memory_win32_handle_properties,
            cached_memory_type_index: None,
            imported_image: None,
            pending_copy: None,
            copy_in_flight: false,
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
            get_memory_win32_handle_properties: load_device_fn!(
                "vkGetMemoryWin32HandlePropertiesKHR",
                PfnVkGetMemoryWin32HandlePropertiesKHR
            ),
        }
    }

    fn find_copy_queue(
        lib: &libloading::Library,
        physical_device: vk::PhysicalDevice,
        _fns: &VulkanFunctions,
    ) -> (u32, u32, bool) {
        // Default to Godot's graphics queue (family 0, queue 0)
        let default = (0u32, 0u32, false);

        if physical_device == vk::PhysicalDevice::null() {
            return default;
        }

        // Load instance function to query queue families
        type GetPhysicalDeviceQueueFamilyProperties = unsafe extern "system" fn(
            physical_device: vk::PhysicalDevice,
            p_queue_family_property_count: *mut u32,
            p_queue_family_properties: *mut vk::QueueFamilyProperties,
        );

        let get_queue_family_props: GetPhysicalDeviceQueueFamilyProperties = unsafe {
            match lib.get(b"vkGetPhysicalDeviceQueueFamilyProperties\0") {
                Ok(f) => *f,
                Err(_) => return default,
            }
        };

        // Query number of queue families
        let mut family_count: u32 = 0;
        unsafe {
            get_queue_family_props(physical_device, &mut family_count, std::ptr::null_mut());
        }

        if family_count == 0 {
            return default;
        }

        // Get queue family properties
        let mut family_props = vec![vk::QueueFamilyProperties::default(); family_count as usize];
        unsafe {
            get_queue_family_props(
                physical_device,
                &mut family_count,
                family_props.as_mut_ptr(),
            );
        }

        // Strategy 1: Try to get queue index 1 from graphics family (family 0)
        // Many GPUs have multiple queues in the graphics family
        if !family_props.is_empty() && family_props[0].queue_count > 1 {
            // Try queue index 1 in graphics family
            godot_print!(
                "[AcceleratedOSR/Vulkan] Graphics family has {} queues, trying queue index 1",
                family_props[0].queue_count
            );
            return (0, 1, true);
        }

        // Strategy 2: Find a dedicated transfer queue family
        for (idx, props) in family_props.iter().enumerate() {
            let has_transfer = props.queue_flags.contains(vk::QueueFlags::TRANSFER);
            let has_graphics = props.queue_flags.contains(vk::QueueFlags::GRAPHICS);
            let has_compute = props.queue_flags.contains(vk::QueueFlags::COMPUTE);

            // Prefer a transfer-only or transfer+compute family (not graphics)
            if has_transfer && !has_graphics && props.queue_count > 0 {
                godot_print!(
                    "[AcceleratedOSR/Vulkan] Found dedicated transfer queue family {} (compute={})",
                    idx,
                    has_compute
                );
                return (idx as u32, 0, true);
            }
        }

        // Strategy 3: Fall back to graphics queue 0
        godot_print!(
            "[AcceleratedOSR/Vulkan] No separate queue available, using shared graphics queue"
        );
        default
    }

    pub fn queue_copy(&mut self, info: &cef::AcceleratedPaintInfo) -> Result<(), String> {
        let handle = HANDLE(info.shared_texture_handle);
        if handle.is_invalid() {
            return Err("Source handle is invalid".into());
        }

        let width = info.extra.coded_size.width as u32;
        let height = info.extra.coded_size.height as u32;

        if width == 0 || height == 0 {
            return Err(format!("Invalid source dimensions: {}x{}", width, height));
        }

        // Duplicate the handle so we own it - this is fast and non-blocking
        let duplicated_handle = duplicate_win32_handle(handle)?;

        // Replace any existing pending copy (drop the old one, which closes its handle)
        self.pending_copy = Some(PendingVulkanCopy {
            duplicated_handle,
            width,
            height,
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

        // Wait for any previous in-flight copy to complete before reusing resources
        if self.copy_in_flight {
            self.wait_for_copy()?;
            self.copy_in_flight = false;
        }

        // Import the D3D12 handle as a Vulkan image
        let src_image = self.import_handle_to_image_from_duplicated(
            pending.duplicated_handle,
            pending.width,
            pending.height,
        )?;

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

        // Submit copy command (non-blocking GPU submission)
        self.submit_copy_async(src_image, dst_image, pending.width, pending.height)?;
        self.copy_in_flight = true;

        // Note: We don't close pending.duplicated_handle here because it's now
        // stored in self.imported_image and will be closed when that's freed.
        // We need to prevent the Drop impl from closing it.
        std::mem::forget(pending);

        Ok(())
    }

    pub fn wait_for_copy(&mut self) -> Result<(), String> {
        if !self.copy_in_flight {
            return Ok(());
        }

        let fns = VULKAN_FNS.get().ok_or("Vulkan functions not loaded")?;
        let result =
            unsafe { (fns.wait_for_fences)(self.device, 1, &self.fence, vk::TRUE, u64::MAX) };
        if result != vk::Result::SUCCESS {
            return Err(format!("Failed to wait for fence: {:?}", result));
        }
        self.copy_in_flight = false;
        Ok(())
    }

    fn import_handle_to_image_from_duplicated(
        &mut self,
        duplicated_handle: HANDLE,
        width: u32,
        height: u32,
    ) -> Result<vk::Image, String> {
        let fns = VULKAN_FNS.get().ok_or("Vulkan functions not loaded")?;

        // Always free previous image - we get a new handle every frame
        self.free_imported_image();

        // Create new image with external memory flag
        let mut external_memory_info = vk::ExternalMemoryImageCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::D3D12_RESOURCE);

        let image_info = vk::ImageCreateInfo::default()
            .push_next(&mut external_memory_info)
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::B8G8R8A8_SRGB)
            .extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);

        let mut image = vk::Image::null();
        let result =
            unsafe { (fns.create_image)(self.device, &image_info, std::ptr::null(), &mut image) };
        if result != vk::Result::SUCCESS {
            return Err(format!("Failed to create image: {:?}", result));
        }

        // Import memory using the duplicated handle
        let memory = match self.import_memory_for_image(duplicated_handle, image, width, height) {
            Ok(mem) => mem,
            Err(e) => {
                unsafe {
                    (fns.destroy_image)(self.device, image, std::ptr::null());
                }
                return Err(e);
            }
        };

        self.imported_image = Some(ImportedVulkanImage {
            duplicated_handle,
            image,
            memory,
        });
        Ok(image)
    }

    fn import_memory_for_image(
        &mut self,
        handle: HANDLE,
        image: vk::Image,
        width: u32,
        height: u32,
    ) -> Result<vk::DeviceMemory, String> {
        let fns = VULKAN_FNS.get().ok_or("Vulkan functions not loaded")?;

        // Get or cache the memory type index (same for all D3D12 imports)
        let memory_type_index = if let Some(cached) = self.cached_memory_type_index {
            cached
        } else {
            // Query memory properties for this handle (only once)
            let mut handle_props = vk::MemoryWin32HandlePropertiesKHR::default();
            let result = unsafe {
                (self.get_memory_win32_handle_properties)(
                    self.device,
                    vk::ExternalMemoryHandleTypeFlags::D3D12_RESOURCE,
                    handle,
                    &mut handle_props,
                )
            };
            if result != vk::Result::SUCCESS {
                return Err(format!(
                    "Failed to get memory handle properties: {:?}",
                    result
                ));
            }

            let idx = Self::find_memory_type_index(handle_props.memory_type_bits)
                .ok_or("Failed to find suitable memory type")?;
            self.cached_memory_type_index = Some(idx);
            idx
        };

        // Import the memory with the Win32 handle
        let mut import_info = vk::ImportMemoryWin32HandleInfoKHR::default()
            .handle_type(vk::ExternalMemoryHandleTypeFlags::D3D12_RESOURCE)
            .handle(handle.0 as isize);

        let mut dedicated_info = vk::MemoryDedicatedAllocateInfo::default().image(image);

        let allocation_size = (width as u64) * (height as u64) * 4;

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

    fn submit_copy_async(
        &mut self,
        src: vk::Image,
        dst: vk::Image,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        let fns = VULKAN_FNS.get().ok_or("Vulkan functions not loaded")?;

        let fence = self.fence;
        let cmd_buffer = self.command_buffer;

        // Reset fence and command buffer
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
        // If using a different queue family, we need to release ownership
        let (src_family, dst_family) = if self.uses_separate_queue && self.queue_family_index != 0 {
            // Release ownership from our transfer queue to graphics queue (family 0)
            (self.queue_family_index, 0u32)
        } else {
            (vk::QUEUE_FAMILY_IGNORED, vk::QUEUE_FAMILY_IGNORED)
        };

        let final_barrier = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .src_queue_family_index(src_family)
            .dst_queue_family_index(dst_family)
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

        // Submit (non-blocking - fence will be signaled when complete)
        let submit_info =
            vk::SubmitInfo::default().command_buffers(std::slice::from_ref(&cmd_buffer));

        let result = unsafe { (fns.queue_submit)(self.queue, 1, &submit_info, fence) };
        if result != vk::Result::SUCCESS {
            return Err(format!("Failed to submit copy command: {:?}", result));
        }

        Ok(())
    }

    fn free_imported_image(&mut self) {
        if let Some(img) = self.imported_image.take()
            && let Some(fns) = VULKAN_FNS.get()
        {
            unsafe {
                (fns.destroy_image)(self.device, img.image, std::ptr::null());
                (fns.free_memory)(self.device, img.memory, std::ptr::null());
                let _ = CloseHandle(img.duplicated_handle);
            }
        }
    }
}

impl Drop for VulkanTextureImporter {
    fn drop(&mut self) {
        if self.copy_in_flight {
            let _ = self.wait_for_copy();
        }

        self.pending_copy = None;

        self.free_imported_image();

        if let Some(fns) = VULKAN_FNS.get() {
            unsafe {
                (fns.destroy_fence)(self.device, self.fence, std::ptr::null());
                (fns.destroy_command_pool)(self.device, self.command_pool, std::ptr::null());
            }
        }
        // Note: device is owned by Godot, don't destroy it
    }
}

unsafe impl Send for VulkanTextureImporter {}
unsafe impl Sync for VulkanTextureImporter {}

/// Get the GPU vendor and device IDs from Godot's Vulkan physical device.
pub fn get_godot_gpu_device_ids() -> Option<(u32, u32)> {
    let mut rd = RenderingServer::singleton().get_rendering_device()?;

    let physical_device_ptr =
        rd.get_driver_resource(DriverResource::PHYSICAL_DEVICE, Rid::Invalid, 0);
    if physical_device_ptr == 0 {
        godot_error!(
            "[AcceleratedOSR/Vulkan] Failed to get Vulkan physical device for GPU ID query"
        );
        return None;
    }
    let physical_device: vk::PhysicalDevice = unsafe { std::mem::transmute(physical_device_ptr) };

    let lib = match unsafe { libloading::Library::new("vulkan-1.dll") } {
        Ok(lib) => lib,
        Err(e) => {
            godot_error!(
                "[AcceleratedOSR/Vulkan] Failed to load vulkan-1.dll for GPU ID query: {}",
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
                     Vulkan 1.1+ is required for GPU ID query.",
                    e
                );
                return None;
            }
        }
    };

    let mut props2 = vk::PhysicalDeviceProperties2::default();

    unsafe {
        get_physical_device_properties2(physical_device, &mut props2);
    }

    let vendor_id = props2.properties.vendor_id;
    let device_id = props2.properties.device_id;
    let device_name = unsafe {
        std::ffi::CStr::from_ptr(props2.properties.device_name.as_ptr())
            .to_string_lossy()
            .into_owned()
    };

    godot_print!(
        "[AcceleratedOSR/Vulkan] Godot GPU: vendor=0x{:04x}, device=0x{:04x}, name={}",
        vendor_id,
        device_id,
        device_name
    );

    Some((vendor_id, device_id))
}
