#[cfg(target_os = "linux")]
macro_rules! impl_vulkan_common_methods {
    (
        memory_field: $memory_field:ident,
        memory_fn_name: $memory_fn_name:literal,
        memory_fn_type: $memory_fn_type:ty
    ) => {
        fn load_vulkan_functions(
            lib: &libloading::Library,
            device: ash::vk::Device,
        ) -> Result<VulkanFunctions, String> {
            type GetDeviceProcAddr = unsafe extern "system" fn(
                ash::vk::Device,
                *const std::ffi::c_char,
            ) -> ash::vk::PFN_vkVoidFunction;

            let get_device_proc_addr: GetDeviceProcAddr = unsafe {
                *lib.get(b"vkGetDeviceProcAddr\0")
                    .map_err(|e| format!("Failed to get vkGetDeviceProcAddr: {e}"))?
            };

            macro_rules! load_device_fn {
                ($fn_name:expr, $fn_type:ty) => {
                    unsafe {
                        let ptr = get_device_proc_addr(
                            device,
                            concat!($fn_name, "\0").as_ptr() as *const _,
                        );
                        if ptr.is_none() {
                            return Err(format!("Failed to load Vulkan function: {}", $fn_name));
                        }
                        std::mem::transmute::<ash::vk::PFN_vkVoidFunction, $fn_type>(ptr)
                    }
                };
            }

            Ok(VulkanFunctions {
                destroy_image: load_device_fn!("vkDestroyImage", ash::vk::PFN_vkDestroyImage),
                free_memory: load_device_fn!("vkFreeMemory", ash::vk::PFN_vkFreeMemory),
                allocate_memory: load_device_fn!("vkAllocateMemory", ash::vk::PFN_vkAllocateMemory),
                bind_image_memory: load_device_fn!(
                    "vkBindImageMemory",
                    ash::vk::PFN_vkBindImageMemory
                ),
                create_image: load_device_fn!("vkCreateImage", ash::vk::PFN_vkCreateImage),
                #[cfg(target_os = "linux")]
                get_image_memory_requirements: load_device_fn!(
                    "vkGetImageMemoryRequirements",
                    ash::vk::PFN_vkGetImageMemoryRequirements
                ),
                get_image_subresource_layout: load_device_fn!(
                    "vkGetImageSubresourceLayout",
                    ash::vk::PFN_vkGetImageSubresourceLayout
                ),
                create_command_pool: load_device_fn!(
                    "vkCreateCommandPool",
                    ash::vk::PFN_vkCreateCommandPool
                ),
                destroy_command_pool: load_device_fn!(
                    "vkDestroyCommandPool",
                    ash::vk::PFN_vkDestroyCommandPool
                ),
                allocate_command_buffers: load_device_fn!(
                    "vkAllocateCommandBuffers",
                    ash::vk::PFN_vkAllocateCommandBuffers
                ),
                create_fence: load_device_fn!("vkCreateFence", ash::vk::PFN_vkCreateFence),
                destroy_fence: load_device_fn!("vkDestroyFence", ash::vk::PFN_vkDestroyFence),
                begin_command_buffer: load_device_fn!(
                    "vkBeginCommandBuffer",
                    ash::vk::PFN_vkBeginCommandBuffer
                ),
                end_command_buffer: load_device_fn!(
                    "vkEndCommandBuffer",
                    ash::vk::PFN_vkEndCommandBuffer
                ),
                cmd_pipeline_barrier: load_device_fn!(
                    "vkCmdPipelineBarrier",
                    ash::vk::PFN_vkCmdPipelineBarrier
                ),
                cmd_copy_image: load_device_fn!("vkCmdCopyImage", ash::vk::PFN_vkCmdCopyImage),
                queue_submit: load_device_fn!("vkQueueSubmit", ash::vk::PFN_vkQueueSubmit),
                #[cfg(target_os = "linux")]
                queue_wait_idle: load_device_fn!("vkQueueWaitIdle", ash::vk::PFN_vkQueueWaitIdle),
                wait_for_fences: load_device_fn!("vkWaitForFences", ash::vk::PFN_vkWaitForFences),
                reset_fences: load_device_fn!("vkResetFences", ash::vk::PFN_vkResetFences),
                reset_command_buffer: load_device_fn!(
                    "vkResetCommandBuffer",
                    ash::vk::PFN_vkResetCommandBuffer
                ),
                #[cfg(target_os = "linux")]
                create_semaphore: load_device_fn!(
                    "vkCreateSemaphore",
                    ash::vk::PFN_vkCreateSemaphore
                ),
                #[cfg(target_os = "linux")]
                destroy_semaphore: load_device_fn!(
                    "vkDestroySemaphore",
                    ash::vk::PFN_vkDestroySemaphore
                ),
                #[cfg(target_os = "linux")]
                import_semaphore_fd: load_device_fn!(
                    "vkImportSemaphoreFdKHR",
                    ash::vk::PFN_vkImportSemaphoreFdKHR
                ),
                $memory_field: load_device_fn!($memory_fn_name, $memory_fn_type),
            })
        }
    };
}

#[cfg(target_os = "linux")]
pub(crate) use impl_vulkan_common_methods;

pub(crate) fn find_memory_type_index(type_filter: u32) -> Option<u32> {
    if type_filter == 0 {
        return None;
    }
    Some(type_filter.trailing_zeros())
}

/// Capture a borrowed external image into an initialized, idle snapshot slot.
///
/// Records barriers, image copy, and final transition into `cmd_buffer`,
/// then resets `fence` and `cmd_buffer` as needed and submits the work.
/// `ctx.queue` must be a queue in Godot's graphics family, covered by the queue
/// synchronization hooks. Bootstrap/publication completion proves `dst`
/// has no remaining readers and is in TRANSFER_SRC_OPTIMAL. The final barrier
/// restores that layout and makes writes visible to Godot's later transfer reads
/// on the same queue. A separate capture queue additionally signals a semaphore
/// which the caller must wait on Godot's queue before publication. The caller
/// waits for `fence` before returning the CEF lease.
pub(crate) fn submit_vulkan_copy_async(
    ctx: &VulkanCopyContext,
    cmd_buffer: ash::vk::CommandBuffer,
    fence: ash::vk::Fence,
    src: ash::vk::Image,
    dst: ash::vk::Image,
    width: u32,
    height: u32,
) -> Result<(), String> {
    use ash::vk;

    let check = |result: vk::Result, operation: &str| {
        if result == vk::Result::SUCCESS {
            Ok(())
        } else {
            Err(format!("{operation} failed: {result:?}"))
        }
    };
    check(
        unsafe { (ctx.reset_fences)(ctx.device, 1, &fence) },
        "vkResetFences",
    )?;
    check(
        unsafe { (ctx.reset_command_buffer)(cmd_buffer, vk::CommandBufferResetFlags::empty()) },
        "vkResetCommandBuffer",
    )?;

    let begin_info =
        vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
    check(
        unsafe { (ctx.begin_command_buffer)(cmd_buffer, &begin_info) },
        "vkBeginCommandBuffer",
    )?;

    let subresource_range = vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    };

    let source_from_external_queue = ctx.src_external_queue_family != vk::QUEUE_FAMILY_IGNORED;
    // Imported producer content must never be discarded with UNDEFINED.
    let src_old_layout = vk::ImageLayout::GENERAL;
    let src_src_queue_family = if source_from_external_queue {
        ctx.src_external_queue_family
    } else {
        vk::QUEUE_FAMILY_IGNORED
    };
    let src_dst_queue_family = if source_from_external_queue {
        ctx.queue_family_index
    } else {
        vk::QUEUE_FAMILY_IGNORED
    };

    let barriers = [
        vk::ImageMemoryBarrier::default()
            .old_layout(src_old_layout)
            .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .src_queue_family_index(src_src_queue_family)
            .dst_queue_family_index(src_dst_queue_family)
            .image(src)
            .subresource_range(subresource_range)
            .src_access_mask(vk::AccessFlags::empty())
            .dst_access_mask(vk::AccessFlags::TRANSFER_READ),
        vk::ImageMemoryBarrier::default()
            // The bootstrap readback (or previous publication completion) proves
            // the staging image is idle in this exact Godot-tracked layout.
            .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(dst)
            .subresource_range(subresource_range)
            .src_access_mask(vk::AccessFlags::TRANSFER_READ)
            .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE),
    ];

    unsafe {
        (ctx.cmd_pipeline_barrier)(
            cmd_buffer,
            vk::PipelineStageFlags::ALL_COMMANDS,
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

    let subresource_layers = vk::ImageSubresourceLayers {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        mip_level: 0,
        base_array_layer: 0,
        layer_count: 1,
    };
    let region = vk::ImageCopy {
        src_subresource: subresource_layers,
        src_offset: vk::Offset3D::default(),
        dst_subresource: subresource_layers,
        dst_offset: vk::Offset3D::default(),
        extent: vk::Extent3D {
            width,
            height,
            depth: 1,
        },
    };

    unsafe {
        (ctx.cmd_copy_image)(
            cmd_buffer,
            src,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            dst,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            1,
            &region,
        );
    }

    if source_from_external_queue {
        let source_release_barrier = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .new_layout(vk::ImageLayout::GENERAL)
            .src_queue_family_index(ctx.queue_family_index)
            .dst_queue_family_index(ctx.src_external_queue_family)
            .image(src)
            .subresource_range(subresource_range)
            .src_access_mask(vk::AccessFlags::TRANSFER_READ)
            .dst_access_mask(vk::AccessFlags::empty());

        unsafe {
            (ctx.cmd_pipeline_barrier)(
                cmd_buffer,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::DependencyFlags::empty(),
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
                1,
                &source_release_barrier,
            );
        }
    }

    let final_barrier = vk::ImageMemoryBarrier::default()
        .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(dst)
        .subresource_range(subresource_range)
        .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .dst_access_mask(vk::AccessFlags::TRANSFER_READ);

    unsafe {
        (ctx.cmd_pipeline_barrier)(
            cmd_buffer,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            1,
            &final_barrier,
        );
    }

    check(
        unsafe { (ctx.end_command_buffer)(cmd_buffer) },
        "vkEndCommandBuffer",
    )?;

    // The FOREIGN_EXT/EXTERNAL acquire and its layout transition precede the
    // first TRANSFER command; the producer wait must gate them too.
    let wait_stages = [vk::PipelineStageFlags::ALL_COMMANDS];
    let waits = [ctx.wait_semaphore];
    let signals = [ctx.signal_semaphore];
    let mut submit_info =
        vk::SubmitInfo::default().command_buffers(std::slice::from_ref(&cmd_buffer));
    if ctx.wait_semaphore != vk::Semaphore::null() {
        submit_info = submit_info
            .wait_semaphores(&waits)
            .wait_dst_stage_mask(&wait_stages);
    }
    if ctx.signal_semaphore != vk::Semaphore::null() {
        submit_info = submit_info.signal_semaphores(&signals);
    }
    let result = unsafe { (ctx.queue_submit)(ctx.queue, 1, &submit_info, fence) };
    if result != vk::Result::SUCCESS {
        return Err(format!("Failed to submit copy command: {:?}", result));
    }

    Ok(())
}

/// Bundles Vulkan function pointers and device-level handles needed for the
/// shared image-copy submission so individual parameters stay manageable.
pub(crate) struct VulkanCopyContext {
    pub device: ash::vk::Device,
    pub queue: ash::vk::Queue,
    pub queue_family_index: u32,
    pub src_external_queue_family: u32,
    /// Optional imported producer completion, used for Linux DMA-BUF sync files.
    pub wait_semaphore: ash::vk::Semaphore,
    /// Optional capture completion for publication on a different physical queue.
    pub signal_semaphore: ash::vk::Semaphore,
    // Function pointers
    pub reset_fences: ash::vk::PFN_vkResetFences,
    pub reset_command_buffer: ash::vk::PFN_vkResetCommandBuffer,
    pub begin_command_buffer: ash::vk::PFN_vkBeginCommandBuffer,
    pub end_command_buffer: ash::vk::PFN_vkEndCommandBuffer,
    pub cmd_pipeline_barrier: ash::vk::PFN_vkCmdPipelineBarrier,
    pub cmd_copy_image: ash::vk::PFN_vkCmdCopyImage,
    pub queue_submit: ash::vk::PFN_vkQueueSubmit,
}

pub(crate) fn get_godot_gpu_device_ids_vulkan(vulkan_lib_name: &str) -> Option<(u32, u32)> {
    use ash::vk;
    use godot::classes::RenderingServer;
    use godot::classes::rendering_device::DriverResource;
    use godot::global::{godot_error, godot_print};
    use godot::prelude::*;

    let rd = RenderingServer::singleton().get_rendering_device()?;

    let physical_device_ptr =
        rd.get_driver_resource(DriverResource::PHYSICAL_DEVICE, Rid::Invalid, 0);
    if physical_device_ptr == 0 {
        godot_error!(
            "[AcceleratedOSR/Vulkan] Failed to get Vulkan physical device for GPU ID query"
        );
        return None;
    }
    let physical_device: vk::PhysicalDevice = unsafe { std::mem::transmute(physical_device_ptr) };

    let lib = match unsafe { libloading::Library::new(vulkan_lib_name) } {
        Ok(lib) => lib,
        Err(e) => {
            godot_error!(
                "[AcceleratedOSR/Vulkan] Failed to load {} for GPU ID query: {}",
                vulkan_lib_name,
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

#[cfg(test)]
mod tests {
    use super::*;
    use ash::vk::{self, Handle};
    use std::cell::RefCell;

    type Barrier = (vk::Image, vk::ImageLayout, vk::ImageLayout, u32, u32);

    #[derive(Default)]
    struct Recorded {
        barriers: Vec<Barrier>,
        wait_stages: Vec<vk::PipelineStageFlags>,
    }

    thread_local! {
        static RECORDED: RefCell<Recorded> = RefCell::default();
    }

    unsafe extern "system" fn ok_reset_fences(
        _: vk::Device,
        _: u32,
        _: *const vk::Fence,
    ) -> vk::Result {
        vk::Result::SUCCESS
    }
    unsafe extern "system" fn ok_reset_command_buffer(
        _: vk::CommandBuffer,
        _: vk::CommandBufferResetFlags,
    ) -> vk::Result {
        vk::Result::SUCCESS
    }
    unsafe extern "system" fn ok_begin(
        _: vk::CommandBuffer,
        _: *const vk::CommandBufferBeginInfo<'_>,
    ) -> vk::Result {
        vk::Result::SUCCESS
    }
    unsafe extern "system" fn ok_end(_: vk::CommandBuffer) -> vk::Result {
        vk::Result::SUCCESS
    }
    #[allow(clippy::too_many_arguments)]
    unsafe extern "system" fn record_barrier(
        _: vk::CommandBuffer,
        _: vk::PipelineStageFlags,
        _: vk::PipelineStageFlags,
        _: vk::DependencyFlags,
        _: u32,
        _: *const vk::MemoryBarrier<'_>,
        _: u32,
        _: *const vk::BufferMemoryBarrier<'_>,
        count: u32,
        barriers: *const vk::ImageMemoryBarrier<'_>,
    ) {
        let barriers = unsafe { std::slice::from_raw_parts(barriers, count as usize) };
        RECORDED.with(|recorded| {
            recorded
                .borrow_mut()
                .barriers
                .extend(barriers.iter().map(|b| {
                    (
                        b.image,
                        b.old_layout,
                        b.new_layout,
                        b.src_queue_family_index,
                        b.dst_queue_family_index,
                    )
                }));
        });
    }
    unsafe extern "system" fn noop_copy(
        _: vk::CommandBuffer,
        _: vk::Image,
        _: vk::ImageLayout,
        _: vk::Image,
        _: vk::ImageLayout,
        _: u32,
        _: *const vk::ImageCopy,
    ) {
    }
    unsafe extern "system" fn record_submit(
        _: vk::Queue,
        count: u32,
        submits: *const vk::SubmitInfo<'_>,
        _: vk::Fence,
    ) -> vk::Result {
        for submit in unsafe { std::slice::from_raw_parts(submits, count as usize) } {
            // Without a wait the stage pointer is null: not a valid empty slice.
            if submit.wait_semaphore_count == 0 {
                continue;
            }
            let stages = unsafe {
                std::slice::from_raw_parts(
                    submit.p_wait_dst_stage_mask,
                    submit.wait_semaphore_count as usize,
                )
            };
            RECORDED.with(|recorded| recorded.borrow_mut().wait_stages.extend_from_slice(stages));
        }
        vk::Result::SUCCESS
    }

    const SRC: u64 = 1;
    const DST: u64 = 2;
    const FAMILY: u32 = 3;

    /// Records one capture of a producer-owned image, as Linux submits it.
    fn record(wait_semaphore: vk::Semaphore) -> Recorded {
        RECORDED.with(|recorded| *recorded.borrow_mut() = Recorded::default());
        let ctx = VulkanCopyContext {
            device: vk::Device::null(),
            queue: vk::Queue::null(),
            queue_family_index: FAMILY,
            src_external_queue_family: vk::QUEUE_FAMILY_FOREIGN_EXT,
            wait_semaphore,
            signal_semaphore: vk::Semaphore::null(),
            reset_fences: ok_reset_fences,
            reset_command_buffer: ok_reset_command_buffer,
            begin_command_buffer: ok_begin,
            end_command_buffer: ok_end,
            cmd_pipeline_barrier: record_barrier,
            cmd_copy_image: noop_copy,
            queue_submit: record_submit,
        };
        let submitted = submit_vulkan_copy_async(
            &ctx,
            vk::CommandBuffer::null(),
            vk::Fence::null(),
            vk::Image::from_raw(SRC),
            vk::Image::from_raw(DST),
            4,
            4,
        );
        assert_eq!(submitted, Ok(()));
        RECORDED.with(|recorded| std::mem::take(&mut *recorded.borrow_mut()))
    }

    #[test]
    fn producer_wait_gates_every_command_including_the_acquire() {
        // The acquire barrier runs before the first TRANSFER-stage command, so
        // a wait limited to TRANSFER would let it race the producer.
        let recorded = record(vk::Semaphore::from_raw(7));
        assert_eq!(recorded.wait_stages, [vk::PipelineStageFlags::ALL_COMMANDS]);
    }

    #[test]
    fn no_wait_is_submitted_without_a_producer_semaphore() {
        assert!(record(vk::Semaphore::null()).wait_stages.is_empty());
    }

    #[test]
    fn producer_image_is_acquired_with_its_contents_and_released_again() {
        let recorded = record(vk::Semaphore::from_raw(7));
        let source: Vec<_> = recorded
            .barriers
            .iter()
            .filter(|barrier| barrier.0 == vk::Image::from_raw(SRC))
            .collect();
        // UNDEFINED as the old layout would let the driver discard the frame.
        assert_eq!(
            source,
            [
                &(
                    vk::Image::from_raw(SRC),
                    vk::ImageLayout::GENERAL,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    vk::QUEUE_FAMILY_FOREIGN_EXT,
                    FAMILY
                ),
                &(
                    vk::Image::from_raw(SRC),
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    vk::ImageLayout::GENERAL,
                    FAMILY,
                    vk::QUEUE_FAMILY_FOREIGN_EXT
                ),
            ]
        );
    }
}
