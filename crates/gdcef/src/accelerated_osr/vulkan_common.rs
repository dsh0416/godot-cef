use ash::vk;
use godot::global::godot_print;

macro_rules! impl_vulkan_common_methods {
    (
        memory_field: $memory_field:ident,
        memory_fn_name: $memory_fn_name:literal,
        memory_fn_type: $memory_fn_type:ty
    ) => {
        fn load_vulkan_functions(lib: &libloading::Library, device: vk::Device) -> VulkanFunctions {
            type GetDeviceProcAddr = unsafe extern "system" fn(
                vk::Device,
                *const std::ffi::c_char,
            ) -> vk::PFN_vkVoidFunction;

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
                $memory_field: load_device_fn!($memory_fn_name, $memory_fn_type),
            }
        }

        fn find_copy_queue(
            lib: &libloading::Library,
            physical_device: vk::PhysicalDevice,
            _fns: &VulkanFunctions,
        ) -> (u32, u32, bool) {
            let default = (0u32, 0u32, false);

            if physical_device == vk::PhysicalDevice::null() {
                return default;
            }

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

            let mut family_count: u32 = 0;
            unsafe {
                get_queue_family_props(physical_device, &mut family_count, std::ptr::null_mut());
            }

            if family_count == 0 {
                return default;
            }

            let mut family_props = vec![vk::QueueFamilyProperties::default(); family_count as usize];
            unsafe {
                get_queue_family_props(
                    physical_device,
                    &mut family_count,
                    family_props.as_mut_ptr(),
                );
            }

            if !family_props.is_empty() && family_props[0].queue_count > 1 {
                godot_print!(
                    "[AcceleratedOSR/Vulkan] Graphics family has {} queues, trying queue index 1",
                    family_props[0].queue_count
                );
                return (0, 1, true);
            }

            for (idx, props) in family_props.iter().enumerate() {
                let has_transfer = props.queue_flags.contains(vk::QueueFlags::TRANSFER);
                let has_graphics = props.queue_flags.contains(vk::QueueFlags::GRAPHICS);
                let has_compute = props.queue_flags.contains(vk::QueueFlags::COMPUTE);

                if has_transfer && !has_graphics && props.queue_count > 0 {
                    godot_print!(
                        "[AcceleratedOSR/Vulkan] Found dedicated transfer queue family {} (compute={})",
                        idx,
                        has_compute
                    );
                    return (idx as u32, 0, true);
                }
            }

            godot_print!(
                "[AcceleratedOSR/Vulkan] No separate queue available, using shared graphics queue"
            );
            default
        }
    };
}

pub(crate) use impl_vulkan_common_methods;
