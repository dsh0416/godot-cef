macro_rules! define_vulkan_hook {
    (
        log_prefix: $log_prefix:literal,
        vulkan_lib: $vulkan_lib:literal,
        status_extension: $status_extension:ident,
        required_extensions: [$($required_extension:ident),+ $(,)?]
    ) => {
        use retour::GenericDetour;
        use std::ffi::{c_void};
        use std::sync::OnceLock;
        use std::sync::atomic::{AtomicBool, Ordering};

        static HOOK_INSTALLED: AtomicBool = AtomicBool::new(false);

        type VkCreateDeviceFn =
            unsafe extern "system" fn(usize, *const c_void, *const c_void, *mut c_void) -> i32;

        static VK_CREATE_DEVICE_HOOK: OnceLock<GenericDetour<VkCreateDeviceFn>> = OnceLock::new();

        #[allow(non_camel_case_types)]
        type PFN_vkEnumerateDeviceExtensionProperties = unsafe extern "system" fn(
            physical_device: ash::vk::PhysicalDevice,
            p_layer_name: *const std::ffi::c_char,
            p_property_count: *mut u32,
            p_properties: *mut ash::vk::ExtensionProperties,
        ) -> ash::vk::Result;

        #[allow(non_camel_case_types)]
        type PFN_vkGetInstanceProcAddr = unsafe extern "system" fn(
            instance: ash::vk::Instance,
            p_name: *const std::ffi::c_char,
        ) -> ash::vk::PFN_vkVoidFunction;

        static ENUMERATE_EXTENSIONS_FN: OnceLock<PFN_vkEnumerateDeviceExtensionProperties> =
            OnceLock::new();

        fn device_supports_extension(
            physical_device: ash::vk::PhysicalDevice,
            extension_name: &std::ffi::CStr,
        ) -> bool {
            let enumerate_fn = match ENUMERATE_EXTENSIONS_FN.get() {
                Some(f) => *f,
                None => return false,
            };

            let mut count: u32 = 0;
            let result = unsafe {
                enumerate_fn(
                    physical_device,
                    std::ptr::null(),
                    &mut count,
                    std::ptr::null_mut(),
                )
            };
            if result != ash::vk::Result::SUCCESS || count == 0 {
                return false;
            }

            let mut properties: Vec<ash::vk::ExtensionProperties> =
                vec![ash::vk::ExtensionProperties::default(); count as usize];
            let result = unsafe {
                enumerate_fn(
                    physical_device,
                    std::ptr::null(),
                    &mut count,
                    properties.as_mut_ptr(),
                )
            };
            if result != ash::vk::Result::SUCCESS {
                return false;
            }

            for prop in &properties {
                let name = unsafe { std::ffi::CStr::from_ptr(prop.extension_name.as_ptr()) };
                if name == extension_name {
                    return true;
                }
            }

            false
        }

        fn extension_already_enabled(
            create_info: &ash::vk::DeviceCreateInfo,
            extension_name: &std::ffi::CStr,
        ) -> bool {
            if create_info.enabled_extension_count == 0
                || create_info.pp_enabled_extension_names.is_null()
            {
                return false;
            }

            let extensions = unsafe {
                std::slice::from_raw_parts(
                    create_info.pp_enabled_extension_names,
                    create_info.enabled_extension_count as usize,
                )
            };

            for &ext_ptr in extensions {
                if !ext_ptr.is_null() {
                    let ext_name = unsafe { std::ffi::CStr::from_ptr(ext_ptr) };
                    if ext_name == extension_name {
                        return true;
                    }
                }
            }

            false
        }

        extern "system" fn hooked_vk_create_device(
            physical_device: usize,
            p_create_info: *const c_void,
            p_allocator: *const c_void,
            p_device: *mut c_void,
        ) -> i32 {
            let hook = VK_CREATE_DEVICE_HOOK.get().expect("Hook not initialized");
            unsafe {
                if p_create_info.is_null() {
                    return hook.call(physical_device, p_create_info, p_allocator, p_device);
                }

                let physical_device_handle =
                    <ash::vk::PhysicalDevice as ash::vk::Handle>::from_raw(physical_device as u64);
                let original_info = &*(p_create_info as *const ash::vk::DeviceCreateInfo<'_>);

                let mut extensions_to_add: Vec<&std::ffi::CStr> = Vec::new();
                $(
                    if device_supports_extension(physical_device_handle, $required_extension)
                        && !extension_already_enabled(original_info, $required_extension)
                    {
                        extensions_to_add.push($required_extension);
                    }
                )+

                if extensions_to_add.is_empty() {
                    if extension_already_enabled(original_info, $status_extension) {
                        eprintln!("{} {} already enabled", $log_prefix, $status_extension.to_string_lossy());
                    } else {
                        eprintln!("{} {} not supported by device", $log_prefix, $status_extension.to_string_lossy());
                    }
                    return hook.call(physical_device, p_create_info, p_allocator, p_device);
                }

                eprintln!("{} Injecting external memory extensions", $log_prefix);

                let original_count = original_info.enabled_extension_count as usize;
                let mut extensions: Vec<*const std::ffi::c_char> =
                    if original_count > 0 && !original_info.pp_enabled_extension_names.is_null() {
                        std::slice::from_raw_parts(
                            original_info.pp_enabled_extension_names,
                            original_count,
                        )
                        .to_vec()
                    } else {
                        Vec::new()
                    };

                for ext in extensions_to_add {
                    eprintln!("{} Adding {}", $log_prefix, ext.to_string_lossy());
                    extensions.push(ext.as_ptr());
                }

                #[allow(deprecated)]
                let modified_info = ash::vk::DeviceCreateInfo {
                    s_type: original_info.s_type,
                    p_next: original_info.p_next,
                    flags: original_info.flags,
                    queue_create_info_count: original_info.queue_create_info_count,
                    p_queue_create_infos: original_info.p_queue_create_infos,
                    enabled_layer_count: original_info.enabled_layer_count,
                    pp_enabled_layer_names: original_info.pp_enabled_layer_names,
                    enabled_extension_count: extensions.len() as u32,
                    pp_enabled_extension_names: extensions.as_ptr(),
                    p_enabled_features: original_info.p_enabled_features,
                    _marker: std::marker::PhantomData,
                };

                let result = hook.call(
                    physical_device,
                    &modified_info as *const _ as *const c_void,
                    p_allocator,
                    p_device,
                );

                let vk_result = ash::vk::Result::from_raw(result);
                if vk_result == ash::vk::Result::SUCCESS {
                    eprintln!(
                        "{} Successfully created device with external memory extensions",
                        $log_prefix
                    );
                } else {
                    eprintln!("{} Device creation failed: {:?}", $log_prefix, vk_result);
                }

                result
            }
        }

        pub fn install_vulkan_hook() {
            if HOOK_INSTALLED.swap(true, Ordering::SeqCst) {
                eprintln!("{} Hook already installed", $log_prefix);
                return;
            }

            eprintln!("{} Installing vkCreateDevice hook...", $log_prefix);

            let lib = unsafe { libloading::Library::new($vulkan_lib) };

            let lib = match lib {
                Ok(lib) => lib,
                Err(e) => {
                    eprintln!("{} Failed to load Vulkan library: {}", $log_prefix, e);
                    HOOK_INSTALLED.store(false, Ordering::SeqCst);
                    return;
                }
            };

            unsafe {
                let get_instance_proc_addr: PFN_vkGetInstanceProcAddr =
                    match lib.get(b"vkGetInstanceProcAddr\0") {
                        Ok(f) => *f,
                        Err(e) => {
                            eprintln!(
                                "{} Failed to get vkGetInstanceProcAddr: {}",
                                $log_prefix, e
                            );
                            HOOK_INSTALLED.store(false, Ordering::SeqCst);
                            return;
                        }
                    };

                let vk_create_device_name = b"vkCreateDevice\0";
                let vk_create_device_ptr = get_instance_proc_addr(
                    ash::vk::Instance::null(),
                    vk_create_device_name.as_ptr() as *const std::ffi::c_char,
                );

                let vk_create_device_fn: VkCreateDeviceFn = if vk_create_device_ptr.is_none() {
                    let vk_create_device: Result<libloading::Symbol<VkCreateDeviceFn>, _> =
                        lib.get(b"vkCreateDevice\0");

                    match vk_create_device {
                        Ok(f) => *f,
                        Err(e) => {
                            eprintln!("{} Failed to get vkCreateDevice: {}", $log_prefix, e);
                            HOOK_INSTALLED.store(false, Ordering::SeqCst);
                            return;
                        }
                    }
                } else {
                    std::mem::transmute::<ash::vk::PFN_vkVoidFunction, VkCreateDeviceFn>(
                        vk_create_device_ptr,
                    )
                };

                let hook = match GenericDetour::new(vk_create_device_fn, hooked_vk_create_device) {
                    Ok(h) => h,
                    Err(e) => {
                        eprintln!("{} Failed to create hook: {}", $log_prefix, e);
                        HOOK_INSTALLED.store(false, Ordering::SeqCst);
                        return;
                    }
                };

                let enumerate_name = b"vkEnumerateDeviceExtensionProperties\0";
                let enumerate_ptr = get_instance_proc_addr(
                    ash::vk::Instance::null(),
                    enumerate_name.as_ptr() as *const std::ffi::c_char,
                );

                if enumerate_ptr.is_some() {
                    let _ = ENUMERATE_EXTENSIONS_FN.set(std::mem::transmute::<
                        ash::vk::PFN_vkVoidFunction,
                        PFN_vkEnumerateDeviceExtensionProperties,
                    >(enumerate_ptr));
                } else if let Ok(f) =
                    lib.get::<PFN_vkEnumerateDeviceExtensionProperties>(
                        b"vkEnumerateDeviceExtensionProperties\0",
                    )
                {
                    let _ = ENUMERATE_EXTENSIONS_FN.set(*f);
                }

                if let Err(e) = hook.enable() {
                    eprintln!("{} Failed to enable hook: {}", $log_prefix, e);
                    HOOK_INSTALLED.store(false, Ordering::SeqCst);
                    return;
                }

                if VK_CREATE_DEVICE_HOOK.set(hook).is_err() {
                    eprintln!("{} Hook already stored (this shouldn't happen)", $log_prefix);
                }

                std::mem::forget(lib);
                eprintln!("{} Hook installed successfully", $log_prefix);
            }
        }
    };
}

pub(crate) use define_vulkan_hook;
