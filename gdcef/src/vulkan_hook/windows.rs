//! Windows-specific Vulkan hook implementation.
//!
//! On Windows, we need to inject `VK_KHR_external_memory_win32` to enable
//! sharing textures via Windows HANDLEs between Godot and CEF.

use ash::vk;
use retour::static_detour;
use std::ffi::{CStr, c_char, c_void};
use std::sync::atomic::{AtomicBool, Ordering};

static HOOK_INSTALLED: AtomicBool = AtomicBool::new(false);

/// The extension we want to inject for Windows HANDLE-based memory sharing
const VK_KHR_EXTERNAL_MEMORY_WIN32_NAME: &CStr =
    unsafe { CStr::from_bytes_with_nul_unchecked(b"VK_KHR_external_memory_win32\0") };

/// Additional required extension for external memory
const VK_KHR_EXTERNAL_MEMORY_NAME: &CStr =
    unsafe { CStr::from_bytes_with_nul_unchecked(b"VK_KHR_external_memory\0") };

#[allow(non_camel_case_types)]
type PFN_vkCreateDevice = unsafe extern "system" fn(
    physical_device: vk::PhysicalDevice,
    p_create_info: *const vk::DeviceCreateInfo<'_>,
    p_allocator: *const vk::AllocationCallbacks<'_>,
    p_device: *mut vk::Device,
) -> vk::Result;

#[allow(non_camel_case_types)]
type PFN_vkEnumerateDeviceExtensionProperties = unsafe extern "system" fn(
    physical_device: vk::PhysicalDevice,
    p_layer_name: *const c_char,
    p_property_count: *mut u32,
    p_properties: *mut vk::ExtensionProperties,
) -> vk::Result;

#[allow(non_camel_case_types)]
type PFN_vkGetInstanceProcAddr =
    unsafe extern "system" fn(instance: vk::Instance, p_name: *const c_char) -> vk::PFN_vkVoidFunction;

static_detour! {
    static VkCreateDeviceHook: unsafe extern "system" fn(
        vk::PhysicalDevice,
        *const vk::DeviceCreateInfo<'_>,
        *const vk::AllocationCallbacks<'_>,
        *mut vk::Device,
    ) -> vk::Result;
}

/// Global storage for vkEnumerateDeviceExtensionProperties function pointer
static mut ENUMERATE_EXTENSIONS_FN: Option<PFN_vkEnumerateDeviceExtensionProperties> = None;

/// Check if the physical device supports an extension
///
/// # Safety
/// This function accesses global mutable state and calls unsafe Vulkan functions.
unsafe fn device_supports_extension(
    physical_device: vk::PhysicalDevice,
    extension_name: &CStr,
) -> bool {
    let enumerate_fn = match unsafe { ENUMERATE_EXTENSIONS_FN } {
        Some(f) => f,
        None => return false,
    };

    // First call to get count
    let mut count: u32 = 0;
    let result = unsafe { enumerate_fn(physical_device, std::ptr::null(), &mut count, std::ptr::null_mut()) };
    if result != vk::Result::SUCCESS || count == 0 {
        return false;
    }

    // Second call to get properties
    let mut properties: Vec<vk::ExtensionProperties> = vec![vk::ExtensionProperties::default(); count as usize];
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

/// Check if an extension is already enabled in the create info
///
/// # Safety
/// The caller must ensure create_info contains valid pointers.
unsafe fn extension_already_enabled(create_info: &vk::DeviceCreateInfo, extension_name: &CStr) -> bool {
    if create_info.enabled_extension_count == 0 || create_info.pp_enabled_extension_names.is_null() {
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

/// The hooked vkCreateDevice function
fn hooked_vk_create_device(
    physical_device: vk::PhysicalDevice,
    p_create_info: *const vk::DeviceCreateInfo<'_>,
    p_allocator: *const vk::AllocationCallbacks<'_>,
    p_device: *mut vk::Device,
) -> vk::Result {
    unsafe {
        if p_create_info.is_null() {
            return VkCreateDeviceHook.call(physical_device, p_create_info, p_allocator, p_device);
        }

        let original_info = &*p_create_info;

        // Check which extensions we need to inject
        let need_external_memory = device_supports_extension(physical_device, VK_KHR_EXTERNAL_MEMORY_NAME)
            && !extension_already_enabled(original_info, VK_KHR_EXTERNAL_MEMORY_NAME);
        
        let need_external_memory_win32 = device_supports_extension(physical_device, VK_KHR_EXTERNAL_MEMORY_WIN32_NAME)
            && !extension_already_enabled(original_info, VK_KHR_EXTERNAL_MEMORY_WIN32_NAME);

        if !need_external_memory && !need_external_memory_win32 {
            // Either not supported or already enabled
            if extension_already_enabled(original_info, VK_KHR_EXTERNAL_MEMORY_WIN32_NAME) {
                eprintln!("[VulkanHook/Windows] VK_KHR_external_memory_win32 already enabled");
            } else {
                eprintln!("[VulkanHook/Windows] VK_KHR_external_memory_win32 not supported by device");
            }
            return VkCreateDeviceHook.call(physical_device, p_create_info, p_allocator, p_device);
        }

        eprintln!("[VulkanHook/Windows] Injecting external memory extensions");

        // Build new extension list
        let original_count = original_info.enabled_extension_count as usize;
        let mut extensions: Vec<*const c_char> = if original_count > 0 && !original_info.pp_enabled_extension_names.is_null() {
            std::slice::from_raw_parts(
                original_info.pp_enabled_extension_names,
                original_count,
            ).to_vec()
        } else {
            Vec::new()
        };

        // Add our extensions
        if need_external_memory {
            eprintln!("[VulkanHook/Windows] Adding VK_KHR_external_memory");
            extensions.push(VK_KHR_EXTERNAL_MEMORY_NAME.as_ptr());
        }
        if need_external_memory_win32 {
            eprintln!("[VulkanHook/Windows] Adding VK_KHR_external_memory_win32");
            extensions.push(VK_KHR_EXTERNAL_MEMORY_WIN32_NAME.as_ptr());
        }

        // Create a modified DeviceCreateInfo
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

        let result = VkCreateDeviceHook.call(
            physical_device,
            &modified_info as *const _,
            p_allocator,
            p_device,
        );

        if result == vk::Result::SUCCESS {
            eprintln!("[VulkanHook/Windows] Successfully created device with external memory extensions");
        } else {
            eprintln!("[VulkanHook/Windows] Device creation failed: {:?}", result);
        }

        result
    }
}

/// Install the Vulkan hook. Should be called during GDExtension Core initialization.
pub fn install_vulkan_hook() {
    if HOOK_INSTALLED.swap(true, Ordering::SeqCst) {
        eprintln!("[VulkanHook/Windows] Hook already installed");
        return;
    }

    eprintln!("[VulkanHook/Windows] Installing vkCreateDevice hook...");

    // Try to load the Vulkan library
    let lib = unsafe {
        libloading::Library::new("vulkan-1.dll")
    };

    let lib = match lib {
        Ok(lib) => lib,
        Err(e) => {
            eprintln!("[VulkanHook/Windows] Failed to load Vulkan library: {}", e);
            HOOK_INSTALLED.store(false, Ordering::SeqCst);
            return;
        }
    };

    unsafe {
        // Get vkGetInstanceProcAddr first
        let get_instance_proc_addr: PFN_vkGetInstanceProcAddr = match lib.get(b"vkGetInstanceProcAddr\0") {
            Ok(f) => *f,
            Err(e) => {
                eprintln!("[VulkanHook/Windows] Failed to get vkGetInstanceProcAddr: {}", e);
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

        if vk_create_device_ptr.is_none() {
            // Try getting it directly from the library
            let vk_create_device: Result<libloading::Symbol<PFN_vkCreateDevice>, _> =
                lib.get(b"vkCreateDevice\0");
            
            match vk_create_device {
                Ok(f) => {
                    if let Err(e) = VkCreateDeviceHook.initialize(*f, hooked_vk_create_device) {
                        eprintln!("[VulkanHook/Windows] Failed to initialize hook: {}", e);
                        HOOK_INSTALLED.store(false, Ordering::SeqCst);
                        return;
                    }
                }
                Err(e) => {
                    eprintln!("[VulkanHook/Windows] Failed to get vkCreateDevice: {}", e);
                    HOOK_INSTALLED.store(false, Ordering::SeqCst);
                    return;
                }
            }
        } else {
            let vk_create_device: PFN_vkCreateDevice = std::mem::transmute(vk_create_device_ptr);
            if let Err(e) = VkCreateDeviceHook.initialize(vk_create_device, hooked_vk_create_device) {
                eprintln!("[VulkanHook/Windows] Failed to initialize hook: {}", e);
                HOOK_INSTALLED.store(false, Ordering::SeqCst);
                return;
            }
        }

        // Get vkEnumerateDeviceExtensionProperties for checking extension support
        let enumerate_name = b"vkEnumerateDeviceExtensionProperties\0";
        let enumerate_ptr = get_instance_proc_addr(
            vk::Instance::null(),
            enumerate_name.as_ptr() as *const c_char,
        );

        if enumerate_ptr.is_some() {
            ENUMERATE_EXTENSIONS_FN = Some(std::mem::transmute(enumerate_ptr));
        } else {
            // Try getting it directly
            if let Ok(f) = lib.get::<PFN_vkEnumerateDeviceExtensionProperties>(b"vkEnumerateDeviceExtensionProperties\0") {
                ENUMERATE_EXTENSIONS_FN = Some(*f);
            }
        }

        // Enable the hook
        if let Err(e) = VkCreateDeviceHook.enable() {
            eprintln!("[VulkanHook/Windows] Failed to enable hook: {}", e);
            HOOK_INSTALLED.store(false, Ordering::SeqCst);
            return;
        }

        // Keep the library loaded for the lifetime of the process
        std::mem::forget(lib);

        eprintln!("[VulkanHook/Windows] Hook installed successfully");
    }
}

