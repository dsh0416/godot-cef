macro_rules! impl_vulkan_common_methods {
    (
        memory_field: $memory_field:ident,
        memory_fn_name: $memory_fn_name:literal,
        memory_fn_type: $memory_fn_type:ty
    ) => {
        fn load_vulkan_functions(
            lib: &libloading::Library,
            device: ash::vk::Device,
        ) -> VulkanFunctions {
            type GetDeviceProcAddr = unsafe extern "system" fn(
                ash::vk::Device,
                *const std::ffi::c_char,
            ) -> ash::vk::PFN_vkVoidFunction;

            let get_device_proc_addr: GetDeviceProcAddr = unsafe {
                *lib.get(b"vkGetDeviceProcAddr\0")
                    .expect("Failed to get vkGetDeviceProcAddr")
            };

            macro_rules! load_device_fn {
                ($fn_name:expr, $fn_type:ty) => {
                    unsafe {
                        let ptr =
                            get_device_proc_addr(device, concat!($fn_name, "\0").as_ptr() as *const _);
                        if ptr.is_none() {
                            panic!("Failed to load Vulkan function: {}", $fn_name);
                        }
                        std::mem::transmute::<ash::vk::PFN_vkVoidFunction, $fn_type>(ptr)
                    }
                };
            }

            VulkanFunctions {
                destroy_image: load_device_fn!("vkDestroyImage", ash::vk::PFN_vkDestroyImage),
                free_memory: load_device_fn!("vkFreeMemory", ash::vk::PFN_vkFreeMemory),
                allocate_memory: load_device_fn!("vkAllocateMemory", ash::vk::PFN_vkAllocateMemory),
                bind_image_memory: load_device_fn!(
                    "vkBindImageMemory",
                    ash::vk::PFN_vkBindImageMemory
                ),
                create_image: load_device_fn!("vkCreateImage", ash::vk::PFN_vkCreateImage),
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
                wait_for_fences: load_device_fn!("vkWaitForFences", ash::vk::PFN_vkWaitForFences),
                reset_fences: load_device_fn!("vkResetFences", ash::vk::PFN_vkResetFences),
                reset_command_buffer: load_device_fn!(
                    "vkResetCommandBuffer",
                    ash::vk::PFN_vkResetCommandBuffer
                ),
                get_device_queue: load_device_fn!("vkGetDeviceQueue", ash::vk::PFN_vkGetDeviceQueue),
                $memory_field: load_device_fn!($memory_fn_name, $memory_fn_type),
            }
        }

        fn find_copy_queue(
            lib: &libloading::Library,
            physical_device: ash::vk::PhysicalDevice,
            _fns: &VulkanFunctions,
        ) -> (u32, u32, bool) {
            let default = (0u32, 0u32, false);

            if physical_device == ash::vk::PhysicalDevice::null() {
                return default;
            }

            type GetPhysicalDeviceQueueFamilyProperties = unsafe extern "system" fn(
                physical_device: ash::vk::PhysicalDevice,
                p_queue_family_property_count: *mut u32,
                p_queue_family_properties: *mut ash::vk::QueueFamilyProperties,
            );

            let get_queue_family_props: GetPhysicalDeviceQueueFamilyProperties = unsafe {
                match lib.get(b"vkGetPhysicalDeviceQueueFamilyProperties\0") {
                    Ok(f) => *f,
                    Err(_) => return default,
                }
            };

            let mut family_count: u32 = 0;
            unsafe {
                get_queue_family_props(physical_device, &mut family_count, std::ptr::null_mut());
            }

            if family_count == 0 {
                return default;
            }

            let mut family_props =
                vec![ash::vk::QueueFamilyProperties::default(); family_count as usize];
            unsafe {
                get_queue_family_props(
                    physical_device,
                    &mut family_count,
                    family_props.as_mut_ptr(),
                );
            }

            if !family_props.is_empty() && family_props[0].queue_count > 1 {
                godot::global::godot_print!(
                    "[AcceleratedOSR/Vulkan] Graphics family has {} queues, trying queue index 1",
                    family_props[0].queue_count
                );
                return (0, 1, true);
            }

            for (idx, props) in family_props.iter().enumerate() {
                let has_transfer = props
                    .queue_flags
                    .contains(ash::vk::QueueFlags::TRANSFER);
                let has_graphics = props
                    .queue_flags
                    .contains(ash::vk::QueueFlags::GRAPHICS);
                let has_compute = props
                    .queue_flags
                    .contains(ash::vk::QueueFlags::COMPUTE);

                if has_transfer && !has_graphics && props.queue_count > 0 {
                    godot::global::godot_print!(
                        "[AcceleratedOSR/Vulkan] Found dedicated transfer queue family {} (compute={})",
                        idx,
                        has_compute
                    );
                    return (idx as u32, 0, true);
                }
            }

            godot::global::godot_print!(
                "[AcceleratedOSR/Vulkan] No separate queue available, using shared graphics queue"
            );
            default
        }
    };
}

pub(crate) use impl_vulkan_common_methods;

pub(crate) fn find_memory_type_index(type_filter: u32) -> Option<u32> {
    if type_filter == 0 {
        return None;
    }
    Some(type_filter.trailing_zeros())
}

/// Shared Vulkan image copy submission.
///
/// Records barriers, image copy, and final transition into `cmd_buffer`,
/// then submits with `fence`. Caller is responsible for resetting fence/cmd_buffer
/// before calling and waiting on the fence afterwards.
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

    let _ = unsafe { (ctx.reset_fences)(ctx.device, 1, &fence) };
    let _ = unsafe { (ctx.reset_command_buffer)(cmd_buffer, vk::CommandBufferResetFlags::empty()) };

    let begin_info =
        vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
    let _ = unsafe { (ctx.begin_command_buffer)(cmd_buffer, &begin_info) };

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
        (ctx.cmd_pipeline_barrier)(
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

    let (src_family, dst_family) = if ctx.uses_separate_queue && ctx.queue_family_index != 0 {
        (ctx.queue_family_index, 0u32)
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
        (ctx.cmd_pipeline_barrier)(
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

    let _ = unsafe { (ctx.end_command_buffer)(cmd_buffer) };

    let submit_info = vk::SubmitInfo::default().command_buffers(std::slice::from_ref(&cmd_buffer));
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
    pub uses_separate_queue: bool,
    pub queue_family_index: u32,
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
