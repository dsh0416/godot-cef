//! Linux capture of a callback-borrowed DMA-BUF into an owned snapshot slot.
//!
//! Imports are deliberately per callback. An inode or duplicated FD identifies
//! storage, not an immutable frame, and CEF's lease ends when capture returns.

use ash::vk::{self, Handle};
use godot::classes::RenderingServer;
use godot::classes::rendering_device::DriverResource;
use godot::global::godot_print;
use godot::prelude::*;
use std::ffi::CStr;
use std::os::fd::RawFd;

use crate::accelerated_osr::vulkan_common::{
    VulkanCopyContext, find_memory_type_index, get_godot_gpu_device_ids_vulkan,
    impl_vulkan_common_methods, submit_vulkan_copy_async,
};
use crate::accelerated_osr::{NativeCaptureTarget, SnapshotFormat};

const DRM_FORMAT_MOD_INVALID: u64 = 0x00ffffffffffffff;

struct DmaBufImportParams {
    fds: Vec<RawFd>,
    strides: Vec<u32>,
    offsets: Vec<u64>,
    modifier: u64,
    format: vk::Format,
    width: u32,
    height: u32,
}

impl Drop for DmaBufImportParams {
    fn drop(&mut self) {
        for &fd in &self.fds {
            if fd >= 0 {
                unsafe { libc::close(fd) };
            }
        }
    }
}

type PfnVkGetMemoryFdPropertiesKHR = unsafe extern "system" fn(
    vk::Device,
    vk::ExternalMemoryHandleTypeFlags,
    RawFd,
    *mut vk::MemoryFdPropertiesKHR<'_>,
) -> vk::Result;
type PfnVkGetPhysicalDeviceImageFormatProperties2 = unsafe extern "system" fn(
    vk::PhysicalDevice,
    *const vk::PhysicalDeviceImageFormatInfo2<'_>,
    *mut vk::ImageFormatProperties2<'_>,
) -> vk::Result;

pub struct VulkanTextureImporter {
    device: vk::Device,
    physical_device: vk::PhysicalDevice,
    command_pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,
    fence: vk::Fence,
    queue: vk::Queue,
    queue_family_index: u32,
    get_memory_fd_properties: PfnVkGetMemoryFdPropertiesKHR,
    get_physical_device_image_format_properties2:
        Option<PfnVkGetPhysicalDeviceImageFormatProperties2>,
    fns: VulkanFunctions,
    device_lost: bool,
    // Keep the dispatch library alive as long as its function pointers.
    _library: libloading::Library,
}

struct ImportedVulkanImage {
    image: vk::Image,
    memory: vk::DeviceMemory,
}

#[derive(Clone, Copy)]
struct VulkanFunctions {
    destroy_image: vk::PFN_vkDestroyImage,
    free_memory: vk::PFN_vkFreeMemory,
    allocate_memory: vk::PFN_vkAllocateMemory,
    bind_image_memory: vk::PFN_vkBindImageMemory,
    create_image: vk::PFN_vkCreateImage,
    get_image_memory_requirements: vk::PFN_vkGetImageMemoryRequirements,
    get_image_subresource_layout: vk::PFN_vkGetImageSubresourceLayout,
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
    queue_wait_idle: vk::PFN_vkQueueWaitIdle,
    wait_for_fences: vk::PFN_vkWaitForFences,
    reset_fences: vk::PFN_vkResetFences,
    reset_command_buffer: vk::PFN_vkResetCommandBuffer,
    get_memory_fd_properties: PfnVkGetMemoryFdPropertiesKHR,
    create_semaphore: vk::PFN_vkCreateSemaphore,
    destroy_semaphore: vk::PFN_vkDestroySemaphore,
    import_semaphore_fd: vk::PFN_vkImportSemaphoreFdKHR,
}

impl VulkanTextureImporter {
    /// Render-thread initialization. Capture never calls Godot's RD API.
    pub fn new() -> Result<Self, String> {
        let rd = RenderingServer::singleton()
            .get_rendering_device()
            .ok_or("RenderingDevice unavailable for Vulkan capture")?;
        let device = vk::Device::from_raw(rd.get_driver_resource(
            DriverResource::LOGICAL_DEVICE,
            Rid::Invalid,
            0,
        ));
        let physical_device = vk::PhysicalDevice::from_raw(rd.get_driver_resource(
            DriverResource::PHYSICAL_DEVICE,
            Rid::Invalid,
            0,
        ));
        let queue = vk::Queue::from_raw(rd.get_driver_resource(
            DriverResource::COMMAND_QUEUE,
            Rid::Invalid,
            0,
        ));
        let queue_family_index =
            u32::try_from(rd.get_driver_resource(DriverResource::QUEUE_FAMILY, Rid::Invalid, 0))
                .map_err(|_| "Invalid Godot Vulkan queue family")?;
        if device == vk::Device::null()
            || physical_device == vk::PhysicalDevice::null()
            || queue == vk::Queue::null()
        {
            return Err("Godot returned a null Vulkan device or graphics queue".into());
        }
        crate::vulkan_hook::queue_sync::ensure_queue_synchronization(device, queue)?;
        let library = unsafe { libloading::Library::new("libvulkan.so.1") }
            .map_err(|error| format!("Failed to load Vulkan: {error}"))?;
        let fns = Self::load_vulkan_functions(&library, device)?;
        let get_physical_device_image_format_properties2 = unsafe {
            *library
                .get::<PfnVkGetPhysicalDeviceImageFormatProperties2>(
                    b"vkGetPhysicalDeviceImageFormatProperties2\0",
                )
                .map_err(|error| format!("Missing Vulkan image format query: {error}"))?
        };
        // CEF's DMA-BUF may originate from a foreign API/driver. EXTERNAL is not
        // an interchangeable fallback for FOREIGN_EXT ownership.
        if !Self::device_supports_extension(
            &library,
            physical_device,
            c"VK_EXT_queue_family_foreign",
        ) {
            return Err("DMA-BUF capture requires VK_EXT_queue_family_foreign".into());
        }
        if !Self::device_supports_extension(
            &library,
            physical_device,
            c"VK_KHR_external_semaphore_fd",
        ) {
            return Err("DMA-BUF capture requires VK_KHR_external_semaphore_fd".into());
        }
        let pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(queue_family_index)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        let mut command_pool = vk::CommandPool::null();
        check(
            unsafe {
                (fns.create_command_pool)(device, &pool_info, std::ptr::null(), &mut command_pool)
            },
            "vkCreateCommandPool",
        )?;
        let allocate = vk::CommandBufferAllocateInfo::default()
            .command_pool(command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let mut command_buffer = vk::CommandBuffer::null();
        if let Err(error) = check(
            unsafe { (fns.allocate_command_buffers)(device, &allocate, &mut command_buffer) },
            "vkAllocateCommandBuffers",
        ) {
            unsafe { (fns.destroy_command_pool)(device, command_pool, std::ptr::null()) };
            return Err(error);
        }
        let mut fence = vk::Fence::null();
        if let Err(error) = check(
            unsafe {
                (fns.create_fence)(
                    device,
                    &vk::FenceCreateInfo::default(),
                    std::ptr::null(),
                    &mut fence,
                )
            },
            "vkCreateFence",
        ) {
            unsafe { (fns.destroy_command_pool)(device, command_pool, std::ptr::null()) };
            return Err(error);
        }
        Ok(Self {
            device,
            physical_device,
            command_pool,
            command_buffer,
            fence,
            queue,
            queue_family_index,
            get_memory_fd_properties: fns.get_memory_fd_properties,
            get_physical_device_image_format_properties2: Some(
                get_physical_device_image_format_properties2,
            ),
            fns,
            device_lost: false,
            _library: library,
        })
    }

    impl_vulkan_common_methods!(
        memory_field: get_memory_fd_properties,
        memory_fn_name: "vkGetMemoryFdPropertiesKHR",
        memory_fn_type: PfnVkGetMemoryFdPropertiesKHR
    );

    pub fn capture(
        &mut self,
        info: &cef::AcceleratedPaintInfo,
        target: NativeCaptureTarget,
    ) -> Result<(), String> {
        self.prepare_publication()?;
        if target.native_handle == 0
            || info.extra.coded_size.width <= 0
            || info.extra.coded_size.height <= 0
            || target.width != info.extra.coded_size.width as u32
            || target.height != info.extra.coded_size.height as u32
        {
            return Err("Vulkan snapshot dimensions do not match CEF's frame".into());
        }
        if SnapshotFormat::from_cef(info)? != target.format {
            return Err("CEF DMA-BUF format does not match its snapshot slot".into());
        }
        let plane_count =
            usize::try_from(info.plane_count).map_err(|_| "Invalid CEF plane count")?;
        if plane_count == 0 || plane_count > info.planes.len() {
            return Err("Invalid CEF DMA-BUF plane count".into());
        }
        let mut params = DmaBufImportParams {
            fds: Vec::with_capacity(plane_count),
            strides: Vec::with_capacity(plane_count),
            offsets: Vec::with_capacity(plane_count),
            modifier: info.modifier,
            format: match target.format {
                SnapshotFormat::Bgra8 => vk::Format::B8G8R8A8_SRGB,
                SnapshotFormat::Rgba8 => vk::Format::R8G8B8A8_SRGB,
            },
            width: target.width,
            height: target.height,
        };
        let mut identity = None;
        for plane in &info.planes[..plane_count] {
            let mut stat = unsafe { std::mem::zeroed::<libc::stat>() };
            if plane.fd < 0 || unsafe { libc::fstat(plane.fd, &mut stat) } != 0 {
                return Err("Invalid CEF DMA-BUF descriptor".into());
            }
            let current = (stat.st_dev, stat.st_ino);
            if identity.is_some_and(|previous| previous != current) {
                return Err("Disjoint DMA-BUF memory planes require a separate import path".into());
            }
            identity = Some(current);
            let duplicated = unsafe { libc::fcntl(plane.fd, libc::F_DUPFD_CLOEXEC, 0) };
            if duplicated < 0 {
                return Err(format!(
                    "Failed to duplicate CEF DMA-BUF: {}",
                    std::io::Error::last_os_error()
                ));
            }
            params.fds.push(duplicated);
            params.strides.push(plane.stride);
            params.offsets.push(plane.offset);
        }
        // Convert Linux's implicit producer fence into an explicit Vulkan wait.
        // CEF keeps the frame leased until the blocking native copy completes.
        let producer = self.import_producer_semaphore(params.fds[0])?;
        let imported = match self.import_dmabuf_to_image(&mut params) {
            Ok(imported) => imported,
            Err(error) => {
                unsafe { (self.fns.destroy_semaphore)(self.device, producer, std::ptr::null()) };
                return Err(error);
            }
        };
        let context = VulkanCopyContext {
            device: self.device,
            queue: self.queue,
            queue_family_index: self.queue_family_index,
            src_external_queue_family: vk::QUEUE_FAMILY_FOREIGN_EXT,
            wait_semaphore: producer,
            reset_fences: self.fns.reset_fences,
            reset_command_buffer: self.fns.reset_command_buffer,
            begin_command_buffer: self.fns.begin_command_buffer,
            end_command_buffer: self.fns.end_command_buffer,
            cmd_pipeline_barrier: self.fns.cmd_pipeline_barrier,
            cmd_copy_image: self.fns.cmd_copy_image,
            queue_submit: self.fns.queue_submit,
        };
        let submitted = submit_vulkan_copy_async(
            &context,
            self.command_buffer,
            self.fence,
            imported.image,
            vk::Image::from_raw(target.native_handle),
            target.width,
            target.height,
        );
        let result = match submitted {
            Ok(()) => self.wait_for_capture(),
            Err(error) => {
                // A submission error is not evidence that the driver retained
                // no commands. Drain before releasing the callback's borrow.
                self.drain_failed_submission();
                Err(error)
            }
        };
        // SUCCESS or terminal device loss is established before returning CEF's
        // borrowed frame. A fence timeout is never treated as completion.
        unsafe {
            (self.fns.destroy_image)(self.device, imported.image, std::ptr::null());
            (self.fns.free_memory)(self.device, imported.memory, std::ptr::null());
            (self.fns.destroy_semaphore)(self.device, producer, std::ptr::null());
        }
        result
    }

    pub fn prepare_publication(&self) -> Result<(), String> {
        if self.device_lost {
            Err("Vulkan device was lost during snapshot capture".into())
        } else {
            // Capture and Godot use the same actual queue. The final transfer
            // barrier establishes visibility for Godot's later RD copy; the
            // shared queue interception serializes host API entry across Godot
            // workers and CEF. No cross-queue host-fence assumption is needed.
            Ok(())
        }
    }

    fn wait_for_capture(&mut self) -> Result<(), String> {
        loop {
            let result = unsafe {
                (self.fns.wait_for_fences)(self.device, 1, &self.fence, vk::TRUE, u64::MAX)
            };
            match result {
                vk::Result::SUCCESS => return Ok(()),
                vk::Result::ERROR_DEVICE_LOST => {
                    self.device_lost = true;
                    return Err("Vulkan device lost while completing CEF capture".into());
                }
                _ => {
                    // Do not return a borrowed CEF resource while submitted work
                    // can still read it, even on a transient fence wait failure.
                    std::thread::yield_now();
                }
            }
        }
    }

    fn drain_failed_submission(&mut self) {
        loop {
            match unsafe { (self.fns.queue_wait_idle)(self.queue) } {
                vk::Result::SUCCESS => return,
                vk::Result::ERROR_DEVICE_LOST => {
                    self.device_lost = true;
                    return;
                }
                _ => std::thread::yield_now(),
            }
        }
    }

    fn import_producer_semaphore(&self, dma_buf: RawFd) -> Result<vk::Semaphore, String> {
        #[repr(C)]
        struct ExportSyncFile {
            flags: u32,
            fd: i32,
        }
        // _IOWR('b', 2, struct dma_buf_export_sync_file), Linux UAPI.
        const DMA_BUF_IOCTL_EXPORT_SYNC_FILE: libc::c_ulong = 0xc008_6202;
        let mut export = ExportSyncFile { flags: 1, fd: -1 }; // DMA_BUF_SYNC_READ
        if unsafe { libc::ioctl(dma_buf, DMA_BUF_IOCTL_EXPORT_SYNC_FILE, &mut export) } < 0 {
            let error = std::io::Error::last_os_error();
            // Kernels predating sync-file export still expose the reservation
            // object's writer fences through DMA-BUF poll(POLLIN). Keep CEF's
            // lease active while waiting, and pair completion with the explicit
            // FOREIGN_EXT acquire barrier recorded by the native copy. This is
            // native fence synchronization; no pixels pass through the CPU.
            if matches!(error.raw_os_error(), Some(libc::ENOTTY | libc::ENOSYS)) {
                wait_for_dma_buf_writer(dma_buf)?;
                return Ok(vk::Semaphore::null());
            }
            return Err(format!("DMA-BUF producer fence export failed: {}", error));
        }
        let mut semaphore = vk::Semaphore::null();
        let created = check(
            unsafe {
                (self.fns.create_semaphore)(
                    self.device,
                    &vk::SemaphoreCreateInfo::default(),
                    std::ptr::null(),
                    &mut semaphore,
                )
            },
            "vkCreateSemaphore",
        );
        if let Err(error) = created {
            if export.fd >= 0 {
                unsafe { libc::close(export.fd) };
            }
            return Err(error);
        }
        let import = vk::ImportSemaphoreFdInfoKHR::default()
            .semaphore(semaphore)
            .flags(vk::SemaphoreImportFlags::TEMPORARY)
            .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD)
            .fd(export.fd);
        let result = unsafe { (self.fns.import_semaphore_fd)(self.device, &import) };
        if let Err(error) = check(result, "vkImportSemaphoreFdKHR") {
            if export.fd >= 0 {
                unsafe { libc::close(export.fd) };
            }
            unsafe { (self.fns.destroy_semaphore)(self.device, semaphore, std::ptr::null()) };
            return Err(error);
        }
        Ok(semaphore) // Vulkan consumed the sync-file descriptor on success.
    }

    fn device_supports_extension(
        library: &libloading::Library,
        physical_device: vk::PhysicalDevice,
        extension: &CStr,
    ) -> bool {
        let Ok(enumerate) = (unsafe {
            library.get::<vk::PFN_vkEnumerateDeviceExtensionProperties>(
                b"vkEnumerateDeviceExtensionProperties\0",
            )
        }) else {
            return false;
        };
        let mut count = 0;
        if unsafe {
            enumerate(
                physical_device,
                std::ptr::null(),
                &mut count,
                std::ptr::null_mut(),
            )
        } != vk::Result::SUCCESS
        {
            return false;
        }
        let mut properties = vec![vk::ExtensionProperties::default(); count as usize];
        if unsafe {
            enumerate(
                physical_device,
                std::ptr::null(),
                &mut count,
                properties.as_mut_ptr(),
            )
        } != vk::Result::SUCCESS
        {
            return false;
        }
        properties.iter().any(
            |property| unsafe { CStr::from_ptr(property.extension_name.as_ptr()) } == extension,
        )
    }

    fn import_dmabuf_to_image(
        &mut self,
        params: &mut DmaBufImportParams,
    ) -> Result<ImportedVulkanImage, String> {
        let fns = self.fns;

        // Create new image with external memory flag for DMA-BUF
        let mut external_memory_info = vk::ExternalMemoryImageCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);

        // VUID-VkImageDrmFormatModifierExplicitCreateInfoEXT-size-02267 requires
        // size == 0: the driver derives each plane's size from this layout.
        let plane_layouts: Vec<vk::SubresourceLayout> = params
            .fds
            .iter()
            .enumerate()
            .map(|(i, _)| {
                let offset = params.offsets.get(i).copied().unwrap_or(0);
                let row_pitch = params.strides.get(i).copied().unwrap_or(0) as u64;
                vk::SubresourceLayout {
                    offset,
                    size: 0,
                    row_pitch,
                    array_pitch: 0,
                    depth_pitch: 0,
                }
            })
            .collect();

        if cfg!(debug_assertions) {
            godot_print!(
                "[AcceleratedOSR/Vulkan] Importing DMA-BUF: format={:?}, size={}x{}, \
                 modifier=0x{:x}, planes={}, strides={:?}, offsets={:?}, plane_layouts={:?}",
                params.format,
                params.width,
                params.height,
                params.modifier,
                params.fds.len(),
                params.strides,
                params.offsets,
                plane_layouts
            );
        }

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

        self.probe_external_image_support(params, tiling)?;

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

        if !use_drm_modifier {
            let subresource =
                vk::ImageSubresource::default().aspect_mask(vk::ImageAspectFlags::COLOR);
            let mut layout = vk::SubresourceLayout::default();
            unsafe {
                (fns.get_image_subresource_layout)(self.device, image, &subresource, &mut layout)
            };
            if params.fds.len() != 1
                || layout.offset != params.offsets[0]
                || layout.row_pitch != u64::from(params.strides[0])
            {
                unsafe { (fns.destroy_image)(self.device, image, std::ptr::null()) };
                return Err(
                    "CEF linear DMA-BUF layout differs from the Vulkan image layout".into(),
                );
            }
        }

        // Import memory for this DMA-BUF
        let memory = match self.import_memory_for_dmabuf(params, image) {
            Ok(mem) => mem,
            Err(e) => {
                unsafe {
                    (fns.destroy_image)(self.device, image, std::ptr::null());
                }
                return Err(e);
            }
        };

        Ok(ImportedVulkanImage { image, memory })
    }

    fn probe_external_image_support(
        &self,
        params: &DmaBufImportParams,
        tiling: vk::ImageTiling,
    ) -> Result<(), String> {
        let Some(get_image_format_properties2) = self.get_physical_device_image_format_properties2
        else {
            return Ok(());
        };
        if self.physical_device == vk::PhysicalDevice::null() {
            return Ok(());
        }

        let mut external_info = vk::PhysicalDeviceExternalImageFormatInfo::default()
            .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);

        let mut modifier_info = vk::PhysicalDeviceImageDrmFormatModifierInfoEXT::default()
            .drm_format_modifier(params.modifier)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);

        let mut format_info = vk::PhysicalDeviceImageFormatInfo2::default()
            .format(params.format)
            .ty(vk::ImageType::TYPE_2D)
            .tiling(tiling)
            .usage(vk::ImageUsageFlags::TRANSFER_SRC)
            .flags(vk::ImageCreateFlags::empty())
            .push_next(&mut external_info);

        if tiling == vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT {
            format_info = format_info.push_next(&mut modifier_info);
        }

        let mut external_props = vk::ExternalImageFormatProperties::default();
        let mut format_props = vk::ImageFormatProperties2 {
            p_next: &mut external_props as *mut _ as *mut _,
            ..Default::default()
        };

        let result = unsafe {
            get_image_format_properties2(self.physical_device, &format_info, &mut format_props)
        };
        if result != vk::Result::SUCCESS {
            return Err(format!(
                "DMA-BUF image format is unsupported before import: {:?} \
                 (format={:?}, tiling={:?}, modifier=0x{:x}, usage={:?})",
                result,
                params.format,
                tiling,
                params.modifier,
                vk::ImageUsageFlags::TRANSFER_SRC
            ));
        }

        let external_memory = external_props.external_memory_properties;
        if !external_memory
            .external_memory_features
            .contains(vk::ExternalMemoryFeatureFlags::IMPORTABLE)
        {
            return Err(format!(
                "DMA-BUF image format is not importable \
                 (format={:?}, tiling={:?}, modifier=0x{:x}, features={:?}, compatible={:?})",
                params.format,
                tiling,
                params.modifier,
                external_memory.external_memory_features,
                external_memory.compatible_handle_types
            ));
        }

        if !external_memory
            .compatible_handle_types
            .contains(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT)
        {
            return Err(format!(
                "DMA-BUF handle type is not compatible with image format \
                 (format={:?}, tiling={:?}, modifier=0x{:x}, compatible={:?})",
                params.format, tiling, params.modifier, external_memory.compatible_handle_types
            ));
        }

        godot_print!(
            "[AcceleratedOSR/Vulkan] DMA-BUF image format probe OK: format={:?}, \
             tiling={:?}, modifier=0x{:x}, max_extent={}x{}, features={:?}, compatible={:?}",
            params.format,
            tiling,
            params.modifier,
            format_props.image_format_properties.max_extent.width,
            format_props.image_format_properties.max_extent.height,
            external_memory.external_memory_features,
            external_memory.compatible_handle_types
        );

        Ok(())
    }

    fn import_memory_for_dmabuf(
        &mut self,
        params: &mut DmaBufImportParams,
        image: vk::Image,
    ) -> Result<vk::DeviceMemory, String> {
        let fns = self.fns;

        // Use the first plane's fd for memory import
        let fd = params.fds[0];

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

        let mut memory_requirements = vk::MemoryRequirements::default();
        unsafe {
            (fns.get_image_memory_requirements)(self.device, image, &mut memory_requirements)
        };

        let memory_type_bits = fd_props.memory_type_bits & memory_requirements.memory_type_bits;
        let memory_type_index = find_memory_type_index(memory_type_bits).ok_or_else(|| {
            format!(
                "Failed to find suitable DMA-BUF memory type \
                 (fd_memory_type_bits=0x{:x}, image_memory_type_bits=0x{:x})",
                fd_props.memory_type_bits, memory_requirements.memory_type_bits
            )
        })?;

        #[cfg(debug_assertions)]
        {
            godot_print!(
                "[AcceleratedOSR/Vulkan] DMA-BUF memory import: \
                 fd_memory_type_bits=0x{:x}, image_memory_type_bits=0x{:x}, \
                 selected_memory_type={}, allocation_size={}, alignment={}",
                fd_props.memory_type_bits,
                memory_requirements.memory_type_bits,
                memory_type_index,
                memory_requirements.size,
                memory_requirements.alignment
            );
        }

        // Import the memory with the DMA-BUF fd
        // Note: The fd ownership is transferred to Vulkan upon successful import
        let mut import_info = vk::ImportMemoryFdInfoKHR::default()
            .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT)
            .fd(fd);

        let mut dedicated_info = vk::MemoryDedicatedAllocateInfo::default().image(image);

        let alloc_info = vk::MemoryAllocateInfo::default()
            .push_next(&mut import_info)
            .push_next(&mut dedicated_info)
            .allocation_size(memory_requirements.size)
            .memory_type_index(memory_type_index);

        let mut memory = vk::DeviceMemory::null();
        let result = unsafe {
            (fns.allocate_memory)(self.device, &alloc_info, std::ptr::null(), &mut memory)
        };
        if result != vk::Result::SUCCESS {
            return Err(format!(
                "Failed to allocate/import DMA-BUF memory: {:?} \
                 (fd_memory_type_bits=0x{:x}, image_memory_type_bits=0x{:x}, \
                 memory_type_index={}, allocation_size={}, alignment={})",
                result,
                fd_props.memory_type_bits,
                memory_requirements.memory_type_bits,
                memory_type_index,
                memory_requirements.size,
                memory_requirements.alignment
            ));
        }

        params.fds[0] = -1;

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
}

impl Drop for VulkanTextureImporter {
    fn drop(&mut self) {
        // Every successful capture completed synchronously; no pending borrow,
        // cached import, command buffer, or semaphore can outlive its callback.
        unsafe {
            (self.fns.destroy_fence)(self.device, self.fence, std::ptr::null());
            (self.fns.destroy_command_pool)(self.device, self.command_pool, std::ptr::null());
        }
    }
}

fn check(result: vk::Result, operation: &str) -> Result<(), String> {
    if result == vk::Result::SUCCESS {
        Ok(())
    } else {
        Err(format!("{operation} failed: {result:?}"))
    }
}

fn wait_for_dma_buf_writer(fd: RawFd) -> Result<(), String> {
    let mut poll_fd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        let result = unsafe { libc::poll(&mut poll_fd, 1, -1) };
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(format!("Failed to wait for DMA-BUF producer: {error}"));
        }
        if poll_fd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            return Err(format!(
                "DMA-BUF producer fence poll failed: {}",
                poll_fd.revents
            ));
        }
        if poll_fd.revents & libc::POLLIN != 0 {
            return Ok(());
        }
    }
}

pub fn get_godot_gpu_device_ids() -> Option<(u32, u32)> {
    get_godot_gpu_device_ids_vulkan("libvulkan.so.1")
}
