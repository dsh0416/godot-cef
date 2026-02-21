//! Windows-specific Vulkan hook implementation.
//!
//! On Windows, we need to inject `VK_KHR_external_memory_win32` to enable
//! sharing textures via Windows HANDLEs between Godot and CEF.

use std::ffi::CStr;

use super::common::define_vulkan_hook;

const VK_KHR_EXTERNAL_MEMORY_NAME: &CStr = c"VK_KHR_external_memory";
const VK_KHR_EXTERNAL_MEMORY_WIN32_NAME: &CStr = c"VK_KHR_external_memory_win32";

define_vulkan_hook!(
    log_prefix: "[VulkanHook/Windows]",
    vulkan_lib: "vulkan-1.dll",
    status_extension: VK_KHR_EXTERNAL_MEMORY_WIN32_NAME,
    required_extensions: [
        VK_KHR_EXTERNAL_MEMORY_NAME,
        VK_KHR_EXTERNAL_MEMORY_WIN32_NAME
    ]
);
