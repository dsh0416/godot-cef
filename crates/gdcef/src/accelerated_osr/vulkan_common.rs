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
