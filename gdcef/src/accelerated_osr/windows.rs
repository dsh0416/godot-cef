use super::RenderBackend;
use ash::vk;
use godot::classes::RenderingServer;
use godot::classes::rendering_device::DriverResource;
use godot::global::{godot_error, godot_print, godot_warn};
use godot::prelude::*;
use std::ffi::c_void;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Graphics::Direct3D12::{
    D3D12_COMMAND_LIST_TYPE_DIRECT, D3D12_COMMAND_QUEUE_DESC, D3D12_RESOURCE_BARRIER,
    D3D12_RESOURCE_BARRIER_0, D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
    D3D12_RESOURCE_BARRIER_FLAG_NONE, D3D12_RESOURCE_BARRIER_TYPE_TRANSITION, D3D12_RESOURCE_DESC,
    D3D12_RESOURCE_DIMENSION_TEXTURE2D, D3D12_RESOURCE_STATE_COMMON,
    D3D12_RESOURCE_STATE_COPY_DEST, D3D12_RESOURCE_TRANSITION_BARRIER, ID3D12CommandAllocator,
    ID3D12CommandQueue, ID3D12Device, ID3D12Fence, ID3D12GraphicsCommandList, ID3D12Resource,
};
use windows::Win32::System::Threading::{CreateEventW, INFINITE, WaitForSingleObject};
use windows::core::Interface;

pub struct D3D12TextureImporter {
    device: std::mem::ManuallyDrop<ID3D12Device>,
    command_queue: ID3D12CommandQueue,
    command_allocator: ID3D12CommandAllocator,
    fence: ID3D12Fence,
    fence_value: u64,
    fence_event: HANDLE,
    device_removed_logged: bool,
}

impl D3D12TextureImporter {
    pub fn new() -> Option<Self> {
        let mut rd = RenderingServer::singleton()
            .get_rendering_device()
            .ok_or_else(|| {
                godot_error!("[AcceleratedOSR/Windows] Failed to get RenderingDevice");
            })
            .ok()?;

        let device_ptr = rd.get_driver_resource(DriverResource::LOGICAL_DEVICE, Rid::Invalid, 0);

        if device_ptr == 0 {
            godot_error!("[AcceleratedOSR/Windows] Failed to get D3D12 device from Godot");
            return None;
        }

        let device: ID3D12Device = unsafe { ID3D12Device::from_raw(device_ptr as *mut c_void) };

        // CRITICAL: Create our OWN command queue instead of using Godot's.
        // Using Godot's command queue causes synchronization conflicts because:
        // 1. Godot is also submitting commands to that queue
        // 2. Our fence signals don't synchronize with Godot's operations
        // 3. This causes DEVICE_HUNG errors on the second frame
        let queue_desc = D3D12_COMMAND_QUEUE_DESC {
            Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
            ..Default::default()
        };
        let command_queue: ID3D12CommandQueue = unsafe { device.CreateCommandQueue(&queue_desc) }
            .map_err(|e| {
                godot_error!(
                    "[AcceleratedOSR/Windows] Failed to create command queue: {:?}",
                    e
                )
            })
            .ok()?;

        // Create command allocator using Godot's device
        let command_allocator: ID3D12CommandAllocator =
            unsafe { device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT) }
                .map_err(|e| {
                    godot_error!(
                        "[AcceleratedOSR/Windows] Failed to create command allocator: {:?}",
                        e
                    )
                })
                .ok()?;

        // Create fence for synchronization
        let fence: ID3D12Fence = unsafe {
            device.CreateFence(
                0,
                windows::Win32::Graphics::Direct3D12::D3D12_FENCE_FLAG_NONE,
            )
        }
        .map_err(|e| godot_error!("[AcceleratedOSR/Windows] Failed to create fence: {:?}", e))
        .ok()?;

        let fence_event = unsafe { CreateEventW(None, false, false, None) }
            .map_err(|e| {
                godot_error!(
                    "[AcceleratedOSR/Windows] Failed to create fence event: {:?}",
                    e
                )
            })
            .ok()?;

        godot_print!("[AcceleratedOSR/Windows] Using Godot's D3D12 device for accelerated OSR");

        Some(Self {
            device: std::mem::ManuallyDrop::new(device),
            command_queue,
            command_allocator,
            fence,
            fence_value: 0,
            fence_event,
            device_removed_logged: false,
        })
    }

    pub fn check_device_state(&mut self) -> Result<(), String> {
        let reason = unsafe { self.device.GetDeviceRemovedReason() };
        if reason.is_ok() {
            self.device_removed_logged = false;
            Ok(())
        } else if !self.device_removed_logged {
            godot_warn!(
                "[AcceleratedOSR/Windows] D3D12 device removed: {:?}",
                reason.err()
            );
            self.device_removed_logged = true;
            Err("D3D12 device removed".into())
        } else {
            Err("D3D12 device removed".into())
        }
    }

    pub fn import_shared_handle(
        &mut self,
        handle: HANDLE,
        _width: u32,
        _height: u32,
        _format: cef::sys::cef_color_type_t,
    ) -> Result<ID3D12Resource, String> {
        if handle.is_invalid() {
            return Err("Shared handle is invalid".into());
        }

        // Open the shared handle to get the D3D12 resource
        let mut resource: Option<ID3D12Resource> = None;
        let result = unsafe { self.device.OpenSharedHandle(handle, &mut resource) };

        if let Err(e) = result {
            let device_reason = unsafe { self.device.GetDeviceRemovedReason() };
            if !self.device_removed_logged {
                if device_reason.is_err() {
                    godot_warn!(
                        "[AcceleratedOSR/Windows] Device removed: {:?}",
                        device_reason.err()
                    );
                } else {
                    godot_warn!("[AcceleratedOSR/Windows] OpenSharedHandle failed: {:?}", e);
                }
                self.device_removed_logged = true;
            }
            return Err("D3D12 device removed".into());
        }

        self.device_removed_logged = false;

        let resource =
            resource.ok_or_else(|| "OpenSharedHandle returned null resource".to_string())?;

        // Validate the resource description
        let desc: D3D12_RESOURCE_DESC = unsafe { resource.GetDesc() };
        if desc.Dimension != D3D12_RESOURCE_DIMENSION_TEXTURE2D {
            return Err(format!(
                "Expected 2D texture, got dimension {:?}",
                desc.Dimension
            ));
        }

        Ok(resource)
    }

    /// Copies the source texture to the destination texture synchronously.
    pub fn copy_texture(
        &mut self,
        src_resource: &ID3D12Resource,
        dst_resource: &ID3D12Resource,
    ) -> Result<(), String> {
        // Wait for previous copy before reusing command allocator
        if self.fence_value > 0 {
            let completed = unsafe { self.fence.GetCompletedValue() };
            if completed < self.fence_value {
                unsafe {
                    self.fence
                        .SetEventOnCompletion(self.fence_value, self.fence_event)
                }
                .map_err(|e| format!("Failed to set event on completion: {:?}", e))?;
                unsafe { WaitForSingleObject(self.fence_event, INFINITE) };
            }
        }

        unsafe { self.command_allocator.Reset() }
            .map_err(|e| format!("Failed to reset command allocator: {:?}", e))?;

        // Create command list
        let command_list: ID3D12GraphicsCommandList = unsafe {
            self.device.CreateCommandList(
                0,
                D3D12_COMMAND_LIST_TYPE_DIRECT,
                &self.command_allocator,
                None,
            )
        }
        .map_err(|e| format!("Failed to create command list: {:?}", e))?;

        // Transition only the destination to COPY_DEST.
        //
        // The source texture is created and fully managed by CEF. CEF keeps the
        // resource in a state suitable for external consumers (typically COMMON)
        // and expects clients not to perform their own state transitions on it.
        // The previous implementation transitioned the source to COPY_SOURCE and
        // back to COMMON, but that interfered with CEF's own resource state
        // tracking. We now rely on CEF's guarantees and leave the source state
        // untouched, transitioning just our destination resource for the copy.
        let dst_barrier = D3D12_RESOURCE_BARRIER {
            Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            Anonymous: D3D12_RESOURCE_BARRIER_0 {
                Transition: std::mem::ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                    pResource: unsafe { std::mem::transmute_copy(dst_resource) },
                    Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                    StateBefore: D3D12_RESOURCE_STATE_COMMON,
                    StateAfter: D3D12_RESOURCE_STATE_COPY_DEST,
                }),
            },
        };

        unsafe { command_list.ResourceBarrier(&[dst_barrier]) };
        unsafe { command_list.CopyResource(dst_resource, src_resource) };

        // Transition back to COMMON for shader read
        let dst_barrier_after = D3D12_RESOURCE_BARRIER {
            Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            Anonymous: D3D12_RESOURCE_BARRIER_0 {
                Transition: std::mem::ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                    pResource: unsafe { std::mem::transmute_copy(dst_resource) },
                    Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                    StateBefore: D3D12_RESOURCE_STATE_COPY_DEST,
                    StateAfter: D3D12_RESOURCE_STATE_COMMON,
                }),
            },
        };

        unsafe { command_list.ResourceBarrier(&[dst_barrier_after]) };

        // Close and execute command list
        unsafe { command_list.Close() }
            .map_err(|e| format!("Failed to close command list: {:?}", e))?;

        let command_lists = [Some(
            command_list
                .cast::<windows::Win32::Graphics::Direct3D12::ID3D12CommandList>()
                .unwrap(),
        )];
        unsafe { self.command_queue.ExecuteCommandLists(&command_lists) };

        self.fence_value += 1;
        unsafe { self.command_queue.Signal(&self.fence, self.fence_value) }
            .map_err(|e| format!("Failed to signal fence: {:?}", e))?;

        // Wait for the copy to complete
        unsafe {
            self.fence
                .SetEventOnCompletion(self.fence_value, self.fence_event)
        }
        .map_err(|e| format!("Failed to set event on completion: {:?}", e))?;
        unsafe { WaitForSingleObject(self.fence_event, INFINITE) };

        Ok(())
    }
}

pub struct GodotTextureImporter {
    backend: TextureImporterBackend,
    current_texture_rid: Option<Rid>,
}

impl GodotTextureImporter {
    pub fn new() -> Option<Self> {
        let render_backend = RenderBackend::detect();

        if !render_backend.supports_accelerated_osr() {
            godot_warn!(
                "[AcceleratedOSR/Windows] Render backend {:?} does not support accelerated OSR. \
                 D3D12 or Vulkan backend is required on Windows.",
                render_backend
            );
            return None;
        }

        let backend = match render_backend {
            RenderBackend::D3D12 => {
                let importer = D3D12TextureImporter::new()?;
                godot_print!("[AcceleratedOSR/Windows] Using D3D12 backend for texture import");
                TextureImporterBackend::D3D12(importer)
            }
            RenderBackend::Vulkan => {
                let importer = VulkanTextureImporter::new()?;
                godot_print!("[AcceleratedOSR/Windows] Using Vulkan backend for texture import");
                TextureImporterBackend::Vulkan(importer)
            }
            _ => {
                godot_warn!(
                    "[AcceleratedOSR/Windows] Unexpected backend {:?}",
                    render_backend
                );
                return None;
            }
        };

        Some(Self {
            backend,
            current_texture_rid: None,
        })
    }

    pub fn import_and_copy(
        &mut self,
        info: &cef::AcceleratedPaintInfo,
        dst_rd_rid: Rid,
    ) -> Result<(), String> {
        match &mut self.backend {
            TextureImporterBackend::D3D12(importer) => {
                Self::import_and_copy_d3d12(importer, info, dst_rd_rid)
            }
            TextureImporterBackend::Vulkan(importer) => {
                importer.import_and_copy(info, dst_rd_rid)
            }
        }
    }

    fn import_and_copy_d3d12(
        importer: &mut D3D12TextureImporter,
        info: &cef::AcceleratedPaintInfo,
        dst_rd_rid: Rid,
    ) -> Result<(), String> {
        importer.check_device_state()?;

        let handle = HANDLE(info.shared_texture_handle);
        if handle.is_invalid() {
            return Err("Source handle is invalid".into());
        }

        let width = info.extra.coded_size.width as u32;
        let height = info.extra.coded_size.height as u32;

        if width == 0 || height == 0 {
            return Err(format!("Invalid source dimensions: {}x{}", width, height));
        }
        if !dst_rd_rid.is_valid() {
            return Err("Destination RID is invalid".into());
        }

        let src_resource = importer.import_shared_handle(
            handle,
            width,
            height,
            *info.format.as_ref(),
        )?;

        // Get destination D3D12 resource from Godot's RenderingDevice
        let dst_resource = {
            let mut rd = RenderingServer::singleton()
                .get_rendering_device()
                .ok_or("Failed to get RenderingDevice")?;

            let resource_ptr = rd.get_driver_resource(DriverResource::TEXTURE, dst_rd_rid, 0);

            if resource_ptr == 0 {
                return Err("Failed to get destination D3D12 resource handle".into());
            }

            unsafe { ID3D12Resource::from_raw(resource_ptr as *mut c_void) }
        };

        importer.copy_texture(&src_resource, &dst_resource)?;

        std::mem::forget(dst_resource);
        Ok(())
    }
}

impl Drop for D3D12TextureImporter {
    fn drop(&mut self) {
        if !self.fence_event.is_invalid() {
            let _ = unsafe { CloseHandle(self.fence_event) };
        }
    }
}

// ============================================================================
// Vulkan Texture Importer (for Vulkan backend on Windows)
// ============================================================================

/// Function pointer types for VK_KHR_external_memory_win32
type PfnVkGetMemoryWin32HandlePropertiesKHR = unsafe extern "system" fn(
    device: vk::Device,
    handle_type: vk::ExternalMemoryHandleTypeFlags,
    handle: HANDLE,
    p_memory_win32_handle_properties: *mut vk::MemoryWin32HandlePropertiesKHR<'_>,
) -> vk::Result;

/// Number of frames to buffer for async operation
const FRAME_BUFFER_COUNT: usize = 2;

pub struct VulkanTextureImporter {
    /// The Vulkan device from Godot (Godot owns it, we just hold the handle)
    device: vk::Device,
    /// Command pool for copy operations
    command_pool: vk::CommandPool,
    /// Double-buffered command buffers
    command_buffers: [vk::CommandBuffer; FRAME_BUFFER_COUNT],
    /// Double-buffered fences
    fences: [vk::Fence; FRAME_BUFFER_COUNT],
    /// Queue for submitting copy commands
    queue: vk::Queue,
    /// Current frame index (alternates 0, 1, 0, 1, ...)
    current_frame: usize,
    /// Function pointer for GetMemoryWin32HandlePropertiesKHR
    get_memory_win32_handle_properties: PfnVkGetMemoryWin32HandlePropertiesKHR,
    /// Cached memory type index for D3D12 imports (avoids querying each frame)
    cached_memory_type_index: Option<u32>,
    /// Cached imported image (reused if handle AND dimensions match)
    imported_image: Option<ImportedVulkanImage>,
}

/// Cached imported image - reused when handle value matches
struct ImportedVulkanImage {
    /// The handle value used to import this image (for cache invalidation)
    handle_value: isize,
    image: vk::Image,
    memory: vk::DeviceMemory,
    extent: vk::Extent2D,
}

/// Vulkan extension function loader
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

/// Static storage for Vulkan function pointers (loaded once)
static VULKAN_FNS: std::sync::OnceLock<VulkanFunctions> = std::sync::OnceLock::new();

impl VulkanTextureImporter {
    pub fn new() -> Option<Self> {
        let mut rd = RenderingServer::singleton().get_rendering_device().ok_or_else(|| {
            godot_error!("[AcceleratedOSR/Vulkan] Failed to get RenderingDevice");
        }).ok()?;

        // Get the Vulkan device from Godot (cast directly to vk::Device which is just a u64 handle)
        let device_ptr = rd.get_driver_resource(DriverResource::LOGICAL_DEVICE, Rid::Invalid, 0);
        if device_ptr == 0 {
            godot_error!("[AcceleratedOSR/Vulkan] Failed to get Vulkan device from Godot");
            return None;
        }
        let device: vk::Device = unsafe { std::mem::transmute(device_ptr as u64) };

        // Load Vulkan library and function pointers
        let lib = match unsafe { libloading::Library::new("vulkan-1.dll") } {
            Ok(lib) => lib,
            Err(e) => {
                godot_error!("[AcceleratedOSR/Vulkan] Failed to load vulkan-1.dll: {}", e);
                return None;
            }
        };

        // Load function pointers using the device
        let fns = VULKAN_FNS.get_or_init(|| {
            Self::load_vulkan_functions(&lib, device)
        });

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
            godot_error!("[AcceleratedOSR/Vulkan] Failed to create command pool: {:?}", result);
            return None;
        }

        // Allocate double-buffered command buffers
        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(FRAME_BUFFER_COUNT as u32);

        let mut command_buffers: [vk::CommandBuffer; FRAME_BUFFER_COUNT] = unsafe { std::mem::zeroed() };
        let result = unsafe {
            (fns.allocate_command_buffers)(device, &alloc_info, command_buffers.as_mut_ptr())
        };
        if result != vk::Result::SUCCESS {
            godot_error!("[AcceleratedOSR/Vulkan] Failed to allocate command buffers: {:?}", result);
            unsafe { (fns.destroy_command_pool)(device, command_pool, std::ptr::null()); }
            return None;
        }

        // Create double-buffered fences (start signaled so first wait doesn't block)
        let fence_info = vk::FenceCreateInfo::default()
            .flags(vk::FenceCreateFlags::SIGNALED);
        let mut fences: [vk::Fence; FRAME_BUFFER_COUNT] = unsafe { std::mem::zeroed() };
        for i in 0..FRAME_BUFFER_COUNT {
            let result = unsafe {
                (fns.create_fence)(device, &fence_info, std::ptr::null(), &mut fences[i])
            };
            if result != vk::Result::SUCCESS {
                godot_error!("[AcceleratedOSR/Vulkan] Failed to create fence: {:?}", result);
                for j in 0..i {
                    unsafe { (fns.destroy_fence)(device, fences[j], std::ptr::null()); }
                }
                unsafe { (fns.destroy_command_pool)(device, command_pool, std::ptr::null()); }
                return None;
            }
        }

        // Keep library loaded for the lifetime of the importer
        std::mem::forget(lib);

        godot_print!("[AcceleratedOSR/Vulkan] Using Godot's Vulkan device for accelerated OSR (double-buffered)");

        Some(Self {
            device,
            command_pool,
            command_buffers,
            queue,
            fences,
            current_frame: 0,
            get_memory_win32_handle_properties: fns.get_memory_win32_handle_properties,
            cached_memory_type_index: None,
            imported_image: None,
        })
    }

    fn load_vulkan_functions(lib: &libloading::Library, device: vk::Device) -> VulkanFunctions {
        type GetDeviceProcAddr = unsafe extern "system" fn(vk::Device, *const std::ffi::c_char) -> vk::PFN_vkVoidFunction;

        let get_device_proc_addr: GetDeviceProcAddr = unsafe {
            *lib.get(b"vkGetDeviceProcAddr\0").expect("Failed to get vkGetDeviceProcAddr")
        };

        // Macro to load device functions
        macro_rules! load_device_fn {
            ($fn_name:expr) => {
                unsafe {
                    let ptr = get_device_proc_addr(device, concat!($fn_name, "\0").as_ptr() as *const _);
                    std::mem::transmute(ptr)
                }
            };
        }

        VulkanFunctions {
            destroy_image: load_device_fn!("vkDestroyImage"),
            free_memory: load_device_fn!("vkFreeMemory"),
            allocate_memory: load_device_fn!("vkAllocateMemory"),
            bind_image_memory: load_device_fn!("vkBindImageMemory"),
            create_image: load_device_fn!("vkCreateImage"),
            create_command_pool: load_device_fn!("vkCreateCommandPool"),
            destroy_command_pool: load_device_fn!("vkDestroyCommandPool"),
            allocate_command_buffers: load_device_fn!("vkAllocateCommandBuffers"),
            create_fence: load_device_fn!("vkCreateFence"),
            destroy_fence: load_device_fn!("vkDestroyFence"),
            begin_command_buffer: load_device_fn!("vkBeginCommandBuffer"),
            end_command_buffer: load_device_fn!("vkEndCommandBuffer"),
            cmd_pipeline_barrier: load_device_fn!("vkCmdPipelineBarrier"),
            cmd_copy_image: load_device_fn!("vkCmdCopyImage"),
            queue_submit: load_device_fn!("vkQueueSubmit"),
            wait_for_fences: load_device_fn!("vkWaitForFences"),
            reset_fences: load_device_fn!("vkResetFences"),
            reset_command_buffer: load_device_fn!("vkResetCommandBuffer"),
            get_device_queue: load_device_fn!("vkGetDeviceQueue"),
            get_memory_win32_handle_properties: load_device_fn!("vkGetMemoryWin32HandlePropertiesKHR"),
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
        let _ = unsafe {
            (fns.wait_for_fences)(self.device, 1, &fence, vk::TRUE, u64::MAX)
        };

        let handle = HANDLE(info.shared_texture_handle);
        if handle.is_invalid() {
            return Err("Source handle is invalid".into());
        }

        let width = info.extra.coded_size.width as u32;
        let height = info.extra.coded_size.height as u32;

        if width == 0 || height == 0 {
            return Err(format!("Invalid source dimensions: {}x{}", width, height));
        }
        if !dst_rd_rid.is_valid() {
            return Err("Destination RID is invalid".into());
        }

        // Import the D3D12 handle as a Vulkan image
        let src_image = self.import_handle_to_image(handle, width, height)?;

        // Get destination Vulkan image from Godot's RenderingDevice
        let dst_image: vk::Image = {
            let mut rd = RenderingServer::singleton()
                .get_rendering_device()
                .ok_or("Failed to get RenderingDevice")?;

            let image_ptr = rd.get_driver_resource(DriverResource::TEXTURE, dst_rd_rid, 0);
            if image_ptr == 0 {
                return Err("Failed to get destination Vulkan image".into());
            }

            unsafe { std::mem::transmute(image_ptr as u64) }
        };

        // Copy from imported image to Godot's texture
        self.submit_copy(src_image, dst_image, width, height, frame_idx)?;

        // Advance to next frame for double buffering
        self.current_frame = (self.current_frame + 1) % FRAME_BUFFER_COUNT;

        Ok(())
    }

    /// Import a D3D12 shared handle into a Vulkan image.
    /// Caches based on handle value - if handle matches, reuse everything.
    /// Note: VkImage can only be bound to memory once, so we must recreate
    /// the image whenever the handle changes.
    fn import_handle_to_image(
        &mut self,
        handle: HANDLE,
        width: u32,
        height: u32,
    ) -> Result<vk::Image, String> {
        let fns = VULKAN_FNS.get().ok_or("Vulkan functions not loaded")?;
        let extent = vk::Extent2D { width, height };
        let handle_value = handle.0 as isize;

        // Check if we can fully reuse existing import (same handle AND dimensions)
        if let Some(existing) = &self.imported_image {
            if existing.handle_value == handle_value && existing.extent == extent {
                // Cache hit! Reuse everything
                return Ok(existing.image);
            }
        }

        // Cache miss - must create new image (VkImage can only be bound once)
        self.free_imported_image();

        // Create new image with external memory flag
        let mut external_memory_info = vk::ExternalMemoryImageCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::D3D12_RESOURCE);

        let image_info = vk::ImageCreateInfo::default()
            .push_next(&mut external_memory_info)
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::B8G8R8A8_SRGB)
            .extent(vk::Extent3D { width, height, depth: 1 })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);

        let mut image = vk::Image::null();
        let result = unsafe {
            (fns.create_image)(self.device, &image_info, std::ptr::null(), &mut image)
        };
        if result != vk::Result::SUCCESS {
            return Err(format!("Failed to create image: {:?}", result));
        }

        // Import memory for this handle
        let memory = self.import_memory_for_image(handle, image, width, height)?;

        self.imported_image = Some(ImportedVulkanImage { 
            handle_value,
            image, 
            memory, 
            extent,
        });
        Ok(image)
    }

    /// Import external memory and bind it to an existing image
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
                return Err(format!("Failed to get memory handle properties: {:?}", result));
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

        let mut dedicated_info = vk::MemoryDedicatedAllocateInfo::default()
            .image(image);

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
        let result = unsafe {
            (fns.bind_image_memory)(self.device, image, memory, 0)
        };
        if result != vk::Result::SUCCESS {
            unsafe { (fns.free_memory)(self.device, memory, std::ptr::null()); }
            return Err(format!("Failed to bind image memory: {:?}", result));
        }

        Ok(memory)
    }

    /// Find the first valid memory type from the type filter bitmask.
    fn find_memory_type_index(type_filter: u32) -> Option<u32> {
        if type_filter == 0 {
            return None;
        }
        Some(type_filter.trailing_zeros())
    }

    /// Submit a copy command without waiting (async)
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
        let _ = unsafe { (fns.reset_command_buffer)(cmd_buffer, vk::CommandBufferResetFlags::empty()) };

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
                0, std::ptr::null(),
                0, std::ptr::null(),
                2, barriers.as_ptr(),
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
            extent: vk::Extent3D { width, height, depth: 1 },
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
                0, std::ptr::null(),
                0, std::ptr::null(),
                1, &final_barrier,
            );
        }

        let _ = unsafe { (fns.end_command_buffer)(cmd_buffer) };

        // Submit without waiting - we'll wait at the start of the next frame's use of this slot
        let submit_info = vk::SubmitInfo::default()
            .command_buffers(std::slice::from_ref(&cmd_buffer));

        let _ = unsafe { (fns.queue_submit)(self.queue, 1, &submit_info, fence) };

        Ok(())
    }

    fn free_imported_image(&mut self) {
        if let Some(img) = self.imported_image.take() {
            if let Some(fns) = VULKAN_FNS.get() {
                unsafe {
                    (fns.destroy_image)(self.device, img.image, std::ptr::null());
                    (fns.free_memory)(self.device, img.memory, std::ptr::null());
                }
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

// ============================================================================
// Backend-agnostic GodotTextureImporter
// ============================================================================

enum TextureImporterBackend {
    D3D12(D3D12TextureImporter),
    Vulkan(VulkanTextureImporter),
}

impl Drop for GodotTextureImporter {
    fn drop(&mut self) {
        if let Some(rid) = self.current_texture_rid.take() {
            RenderingServer::singleton().free_rid(rid);
        }
    }
}

pub fn is_supported() -> bool {
    let backend = RenderBackend::detect();
    if !backend.supports_accelerated_osr() {
        return false;
    }

    match backend {
        RenderBackend::D3D12 => D3D12TextureImporter::new().is_some(),
        RenderBackend::Vulkan => VulkanTextureImporter::new().is_some(),
        _ => false,
    }
}

pub fn get_godot_adapter_luid() -> Option<(i32, u32)> {
    let mut rd = RenderingServer::singleton().get_rendering_device()?;
    let device_ptr = rd.get_driver_resource(DriverResource::LOGICAL_DEVICE, Rid::Invalid, 0);

    if device_ptr == 0 {
        godot_warn!("[AcceleratedOSR/Windows] Failed to get D3D12 device for LUID query");
        return None;
    }

    let device: ID3D12Device = unsafe { ID3D12Device::from_raw(device_ptr as *mut c_void) };
    let luid = unsafe { device.GetAdapterLuid() };
    godot_print!("[AcceleratedOSR/Windows] Godot adapter LUID: {:?}", luid);

    // Device is from Godot, we don't need to close it
    std::mem::forget(device);

    Some((luid.HighPart, luid.LowPart))
}

unsafe impl Send for GodotTextureImporter {}
unsafe impl Sync for GodotTextureImporter {}
