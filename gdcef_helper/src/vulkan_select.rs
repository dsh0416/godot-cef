//! Linux Vulkan device selection based on device UUID.

use std::ffi::CString;

pub fn select_device_by_uuid(target_uuid: [u8; 16]) -> bool {
    let lib = match unsafe { libloading::Library::new("libvulkan.so.1") } {
        Ok(lib) => lib,
        Err(e) => {
            eprintln!("[gdcef_helper/vulkan] Failed to load libvulkan.so.1: {}", e);
            return false;
        }
    };

    let vk_create_instance: VkCreateInstance = unsafe {
        match lib.get(b"vkCreateInstance\0") {
            Ok(f) => *f,
            Err(e) => {
                eprintln!(
                    "[gdcef_helper/vulkan] Failed to get vkCreateInstance: {}",
                    e
                );
                return false;
            }
        }
    };

    let vk_destroy_instance: VkDestroyInstance = unsafe {
        match lib.get(b"vkDestroyInstance\0") {
            Ok(f) => *f,
            Err(e) => {
                eprintln!(
                    "[gdcef_helper/vulkan] Failed to get vkDestroyInstance: {}",
                    e
                );
                return false;
            }
        }
    };

    let vk_enumerate_physical_devices: VkEnumeratePhysicalDevices = unsafe {
        match lib.get(b"vkEnumeratePhysicalDevices\0") {
            Ok(f) => *f,
            Err(e) => {
                eprintln!(
                    "[gdcef_helper/vulkan] Failed to get vkEnumeratePhysicalDevices: {}",
                    e
                );
                return false;
            }
        }
    };

    let vk_get_physical_device_properties2: VkGetPhysicalDeviceProperties2 = unsafe {
        match lib.get(b"vkGetPhysicalDeviceProperties2\0") {
            Ok(f) => *f,
            Err(e) => {
                eprintln!(
                    "[gdcef_helper/vulkan] Failed to get vkGetPhysicalDeviceProperties2: {}",
                    e
                );
                return false;
            }
        }
    };

    let app_name = CString::new("gdcef_helper").unwrap();
    let app_info = VkApplicationInfo {
        s_type: VK_STRUCTURE_TYPE_APPLICATION_INFO,
        p_next: std::ptr::null(),
        p_application_name: app_name.as_ptr(),
        application_version: 1,
        p_engine_name: std::ptr::null(),
        engine_version: 0,
        api_version: VK_API_VERSION_1_1,
    };

    let create_info = VkInstanceCreateInfo {
        s_type: VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
        p_next: std::ptr::null(),
        flags: 0,
        p_application_info: &app_info,
        enabled_layer_count: 0,
        pp_enabled_layer_names: std::ptr::null(),
        enabled_extension_count: 0,
        pp_enabled_extension_names: std::ptr::null(),
    };

    let mut instance: VkInstance = std::ptr::null_mut();
    let result = unsafe { vk_create_instance(&create_info, std::ptr::null(), &mut instance) };
    if result != VK_SUCCESS || instance.is_null() {
        eprintln!(
            "[gdcef_helper/vulkan] Failed to create Vulkan instance: {}",
            result
        );
        return false;
    }

    let mut device_count: u32 = 0;
    let result =
        unsafe { vk_enumerate_physical_devices(instance, &mut device_count, std::ptr::null_mut()) };
    if result != VK_SUCCESS || device_count == 0 {
        eprintln!("[gdcef_helper/vulkan] Failed to enumerate physical devices or no devices found");
        unsafe { vk_destroy_instance(instance, std::ptr::null()) };
        return false;
    }

    let mut physical_devices = vec![std::ptr::null_mut(); device_count as usize];
    let result = unsafe {
        vk_enumerate_physical_devices(instance, &mut device_count, physical_devices.as_mut_ptr())
    };
    if result != VK_SUCCESS {
        eprintln!("[gdcef_helper/vulkan] Failed to get physical devices");
        unsafe { vk_destroy_instance(instance, std::ptr::null()) };
        return false;
    }

    let mut found_index: Option<usize> = None;
    let mut found_vendor_device: Option<(u32, u32)> = None;

    for (idx, &device) in physical_devices.iter().enumerate() {
        let mut id_props = VkPhysicalDeviceIDProperties {
            s_type: VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ID_PROPERTIES,
            p_next: std::ptr::null_mut(),
            device_uuid: [0; 16],
            driver_uuid: [0; 16],
            device_luid: [0; 8],
            device_node_mask: 0,
            device_luid_valid: 0,
        };

        let mut props2 = VkPhysicalDeviceProperties2 {
            s_type: VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES_2,
            p_next: &mut id_props as *mut _ as *mut std::ffi::c_void,
            properties: unsafe { std::mem::zeroed() },
        };

        unsafe { vk_get_physical_device_properties2(device, &mut props2) };

        if id_props.device_uuid == target_uuid {
            found_index = Some(idx);
            found_vendor_device = Some((props2.properties.vendor_id, props2.properties.device_id));
            eprintln!(
                "[gdcef_helper/vulkan] Found matching device at index {}: vendor=0x{:04x}, device=0x{:04x}",
                idx, props2.properties.vendor_id, props2.properties.device_id
            );
            break;
        }
    }

    unsafe { vk_destroy_instance(instance, std::ptr::null()) };

    if let Some(idx) = found_index {
        // MESA_VK_DEVICE_SELECT format: "vendor_id:device_id" or just the index
        // Using vendor:device is more reliable across driver restarts
        if let Some((vendor, device)) = found_vendor_device {
            let select_value = format!("{:04x}:{:04x}", vendor, device);
            unsafe { std::env::set_var("MESA_VK_DEVICE_SELECT", &select_value) };
            eprintln!(
                "[gdcef_helper/vulkan] Set MESA_VK_DEVICE_SELECT={}",
                select_value
            );
        }

        unsafe { std::env::set_var("VK_LOADER_DEVICE_SELECT", idx.to_string()) };
        eprintln!("[gdcef_helper/vulkan] Set VK_LOADER_DEVICE_SELECT={}", idx);

        true
    } else {
        eprintln!(
            "[gdcef_helper/vulkan] No device found matching UUID {:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            target_uuid[0],
            target_uuid[1],
            target_uuid[2],
            target_uuid[3],
            target_uuid[4],
            target_uuid[5],
            target_uuid[6],
            target_uuid[7],
            target_uuid[8],
            target_uuid[9],
            target_uuid[10],
            target_uuid[11],
            target_uuid[12],
            target_uuid[13],
            target_uuid[14],
            target_uuid[15]
        );
        false
    }
}

// Vulkan type definitions (minimal subset needed)
const VK_SUCCESS: i32 = 0;
const VK_STRUCTURE_TYPE_APPLICATION_INFO: u32 = 0;
const VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO: u32 = 1;
const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES_2: u32 = 1000059001;
const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ID_PROPERTIES: u32 = 1000071004;
const VK_API_VERSION_1_1: u32 = (1 << 22) | (1 << 12);

type VkInstance = *mut std::ffi::c_void;
type VkPhysicalDevice = *mut std::ffi::c_void;

#[repr(C)]
struct VkApplicationInfo {
    s_type: u32,
    p_next: *const std::ffi::c_void,
    p_application_name: *const i8,
    application_version: u32,
    p_engine_name: *const i8,
    engine_version: u32,
    api_version: u32,
}

#[repr(C)]
struct VkInstanceCreateInfo {
    s_type: u32,
    p_next: *const std::ffi::c_void,
    flags: u32,
    p_application_info: *const VkApplicationInfo,
    enabled_layer_count: u32,
    pp_enabled_layer_names: *const *const i8,
    enabled_extension_count: u32,
    pp_enabled_extension_names: *const *const i8,
}

#[repr(C)]
struct VkPhysicalDeviceProperties {
    api_version: u32,
    driver_version: u32,
    vendor_id: u32,
    device_id: u32,
    device_type: u32,
    device_name: [i8; 256],
    pipeline_cache_uuid: [u8; 16],
    limits: [u8; 504], // VkPhysicalDeviceLimits is large, we just need space
    sparse_properties: [u8; 20], // VkPhysicalDeviceSparseProperties
}

#[repr(C)]
struct VkPhysicalDeviceProperties2 {
    s_type: u32,
    p_next: *mut std::ffi::c_void,
    properties: VkPhysicalDeviceProperties,
}

#[repr(C)]
struct VkPhysicalDeviceIDProperties {
    s_type: u32,
    p_next: *mut std::ffi::c_void,
    device_uuid: [u8; 16],
    driver_uuid: [u8; 16],
    device_luid: [u8; 8],
    device_node_mask: u32,
    device_luid_valid: u32,
}

type VkCreateInstance = unsafe extern "C" fn(
    *const VkInstanceCreateInfo,
    *const std::ffi::c_void,
    *mut VkInstance,
) -> i32;

type VkDestroyInstance = unsafe extern "C" fn(VkInstance, *const std::ffi::c_void);

type VkEnumeratePhysicalDevices =
    unsafe extern "C" fn(VkInstance, *mut u32, *mut VkPhysicalDevice) -> i32;

type VkGetPhysicalDeviceProperties2 =
    unsafe extern "C" fn(VkPhysicalDevice, *mut VkPhysicalDeviceProperties2);
