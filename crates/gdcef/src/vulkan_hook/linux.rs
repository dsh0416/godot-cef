//! Linux-specific Vulkan hook implementation.
//!
//! On Linux, we need to inject `VK_KHR_external_memory_fd` and `VK_EXT_external_memory_dma_buf`
//! to enable sharing textures via DMA-BUF file descriptors between Godot and CEF.

use ash::vk::{self, Handle};
use retour::GenericDetour;
use std::ffi::{CStr, c_char, c_void};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

static HOOK_INSTALLED: AtomicBool = AtomicBool::new(false);

type VkCreateDeviceFn =
    unsafe extern "system" fn(usize, *const c_void, *const c_void, *mut c_void) -> i32;

static VK_CREATE_DEVICE_HOOK: OnceLock<GenericDetour<VkCreateDeviceFn>> = OnceLock::new();

// Extension names for Linux DMA-BUF sharing
const VK_KHR_EXTERNAL_MEMORY_NAME: &CStr = c"VK_KHR_external_memory";
const VK_KHR_EXTERNAL_MEMORY_FD_NAME: &CStr = c"VK_KHR_external_memory_fd";
const VK_EXT_EXTERNAL_MEMORY_DMA_BUF_NAME: &CStr = c"VK_EXT_external_memory_dma_buf";

#[allow(non_camel_case_types)]
type PFN_vkEnumerateDeviceExtensionProperties = unsafe extern "system" fn(
    physical_device: vk::PhysicalDevice,
    p_layer_name: *const c_char,
    p_property_count: *mut u32,
    p_properties: *mut vk::ExtensionProperties,
) -> vk::Result;

#[allow(non_camel_case_types)]
type PFN_vkGetInstanceProcAddr = unsafe extern "system" fn(
    instance: vk::Instance,
    p_name: *const c_char,
) -> vk::PFN_vkVoidFunction;

static ENUMERATE_EXTENSIONS_FN: OnceLock<PFN_vkEnumerateDeviceExtensionProperties> =
    OnceLock::new();

fn device_supports_extension(physical_device: vk::PhysicalDevice, extension_name: &CStr) -> bool {
    let enumerate_fn = match ENUMERATE_EXTENSIONS_FN.get() {
        Some(f) => *f,
        None => return false,
    };

    // First call to get count
    let mut count: u32 = 0;
    let result = unsafe {
        enumerate_fn(
            physical_device,
            std::ptr::null(),
            &mut count,
            std::ptr::null_mut(),
        )
    };
    if result != vk::Result::SUCCESS || count == 0 {
        return false;
    }

    // Second call to get properties
    let mut properties: Vec<vk::ExtensionProperties> =
        vec![vk::ExtensionProperties::default(); count as usize];
    let result = unsafe {
        enumerate_fn(
            physical_device,
            std::ptr::null(),
            &mut count,
            properties.as_mut_ptr(),
        )
    };
    if result != vk::Result::SUCCESS {
        return false;
    }

    // Check if our extension is in the list
    for prop in properties.iter() {
        let name = unsafe { CStr::from_ptr(prop.extension_name.as_ptr()) };
        if name == extension_name {
            return true;
        }
    }

    false
}

fn extension_already_enabled(create_info: &vk::DeviceCreateInfo, extension_name: &CStr) -> bool {
    if create_info.enabled_extension_count == 0 || create_info.pp_enabled_extension_names.is_null()
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
            let ext_name = unsafe { CStr::from_ptr(ext_ptr) };
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

        // Cast back to Vulkan types for processing
        let physical_device_handle = vk::PhysicalDevice::from_raw(physical_device as u64);
        let original_info = &*(p_create_info as *const vk::DeviceCreateInfo<'_>);

        // Check which extensions we need to inject
        let need_external_memory =
            device_supports_extension(physical_device_handle, VK_KHR_EXTERNAL_MEMORY_NAME)
                && !extension_already_enabled(original_info, VK_KHR_EXTERNAL_MEMORY_NAME);

        let need_external_memory_fd =
            device_supports_extension(physical_device_handle, VK_KHR_EXTERNAL_MEMORY_FD_NAME)
                && !extension_already_enabled(original_info, VK_KHR_EXTERNAL_MEMORY_FD_NAME);

        let need_dma_buf =
            device_supports_extension(physical_device_handle, VK_EXT_EXTERNAL_MEMORY_DMA_BUF_NAME)
                && !extension_already_enabled(original_info, VK_EXT_EXTERNAL_MEMORY_DMA_BUF_NAME);

        if !need_external_memory && !need_external_memory_fd && !need_dma_buf {
            // Either not supported or already enabled
            if extension_already_enabled(original_info, VK_EXT_EXTERNAL_MEMORY_DMA_BUF_NAME) {
                eprintln!("[VulkanHook/Linux] VK_EXT_external_memory_dma_buf already enabled");
            } else {
                eprintln!(
                    "[VulkanHook/Linux] VK_EXT_external_memory_dma_buf not supported by device"
                );
            }
            return hook.call(physical_device, p_create_info, p_allocator, p_device);
        }

        eprintln!("[VulkanHook/Linux] Injecting external memory extensions");

        // Build new extension list
        let original_count = original_info.enabled_extension_count as usize;
        let mut extensions: Vec<*const c_char> =
            if original_count > 0 && !original_info.pp_enabled_extension_names.is_null() {
                std::slice::from_raw_parts(original_info.pp_enabled_extension_names, original_count)
                    .to_vec()
            } else {
                Vec::new()
            };

        // Add our extensions
        if need_external_memory {
            eprintln!("[VulkanHook/Linux] Adding VK_KHR_external_memory");
            extensions.push(VK_KHR_EXTERNAL_MEMORY_NAME.as_ptr());
        }
        if need_external_memory_fd {
            eprintln!("[VulkanHook/Linux] Adding VK_KHR_external_memory_fd");
            extensions.push(VK_KHR_EXTERNAL_MEMORY_FD_NAME.as_ptr());
        }
        if need_dma_buf {
            eprintln!("[VulkanHook/Linux] Adding VK_EXT_external_memory_dma_buf");
            extensions.push(VK_EXT_EXTERNAL_MEMORY_DMA_BUF_NAME.as_ptr());
        }

        // Create a modified DeviceCreateInfo
        // Note: enabled_layer_count and pp_enabled_layer_names are deprecated (device layers no longer operate)
        #[allow(deprecated)]
        let modified_info = vk::DeviceCreateInfo {
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

        let vk_result = vk::Result::from_raw(result);
        if vk_result == vk::Result::SUCCESS {
            eprintln!(
                "[VulkanHook/Linux] Successfully created device with external memory extensions"
            );
        } else {
            eprintln!("[VulkanHook/Linux] Device creation failed: {:?}", vk_result);
        }

        result
    }
}

pub fn install_vulkan_hook() {
    if HOOK_INSTALLED.swap(true, Ordering::SeqCst) {
        eprintln!("[VulkanHook/Linux] Hook already installed");
        return;
    }

    eprintln!("[VulkanHook/Linux] Installing vkCreateDevice hook...");

    // Try to load the Vulkan library
    let lib = unsafe { libloading::Library::new("libvulkan.so.1") };

    let lib = match lib {
        Ok(lib) => lib,
        Err(e) => {
            eprintln!("[VulkanHook/Linux] Failed to load Vulkan library: {}", e);
            HOOK_INSTALLED.store(false, Ordering::SeqCst);
            return;
        }
    };

    unsafe {
        // Get vkGetInstanceProcAddr first
        let get_instance_proc_addr: PFN_vkGetInstanceProcAddr =
            match lib.get(b"vkGetInstanceProcAddr\0") {
                Ok(f) => *f,
                Err(e) => {
                    eprintln!(
                        "[VulkanHook/Linux] Failed to get vkGetInstanceProcAddr: {}",
                        e
                    );
                    HOOK_INSTALLED.store(false, Ordering::SeqCst);
                    return;
                }
            };

        // Get vkCreateDevice - we can get it with a null instance for the loader-level function
        let vk_create_device_name = b"vkCreateDevice\0";
        let vk_create_device_ptr = get_instance_proc_addr(
            vk::Instance::null(),
            vk_create_device_name.as_ptr() as *const c_char,
        );

        let vk_create_device_fn: VkCreateDeviceFn = if vk_create_device_ptr.is_none() {
            // Try getting it directly from the library
            let vk_create_device: Result<libloading::Symbol<VkCreateDeviceFn>, _> =
                lib.get(b"vkCreateDevice\0");

            match vk_create_device {
                Ok(f) => *f,
                Err(e) => {
                    eprintln!("[VulkanHook/Linux] Failed to get vkCreateDevice: {}", e);
                    HOOK_INSTALLED.store(false, Ordering::SeqCst);
                    return;
                }
            }
        } else {
            std::mem::transmute::<vk::PFN_vkVoidFunction, VkCreateDeviceFn>(vk_create_device_ptr)
        };

        // Create and enable the detour
        let hook = match GenericDetour::new(vk_create_device_fn, hooked_vk_create_device) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("[VulkanHook/Linux] Failed to create hook: {}", e);
                HOOK_INSTALLED.store(false, Ordering::SeqCst);
                return;
            }
        };

        // Get vkEnumerateDeviceExtensionProperties for checking extension support
        let enumerate_name = b"vkEnumerateDeviceExtensionProperties\0";
        let enumerate_ptr = get_instance_proc_addr(
            vk::Instance::null(),
            enumerate_name.as_ptr() as *const c_char,
        );

        if enumerate_ptr.is_some() {
            let _ = ENUMERATE_EXTENSIONS_FN.set(std::mem::transmute::<
                vk::PFN_vkVoidFunction,
                PFN_vkEnumerateDeviceExtensionProperties,
            >(enumerate_ptr));
        } else {
            // Try getting it directly
            if let Ok(f) = lib.get::<PFN_vkEnumerateDeviceExtensionProperties>(
                b"vkEnumerateDeviceExtensionProperties\0",
            ) {
                let _ = ENUMERATE_EXTENSIONS_FN.set(*f);
            }
        }

        // Enable the hook
        if let Err(e) = hook.enable() {
            eprintln!("[VulkanHook/Linux] Failed to enable hook: {}", e);
            HOOK_INSTALLED.store(false, Ordering::SeqCst);
            return;
        }

        // Store the hook for later use (and to keep it alive)
        if VK_CREATE_DEVICE_HOOK.set(hook).is_err() {
            eprintln!("[VulkanHook/Linux] Hook already stored (this shouldn't happen)");
        }

        // Keep the library loaded for the lifetime of the process
        std::mem::forget(lib);

        eprintln!("[VulkanHook/Linux] Hook installed successfully");
    }
}
