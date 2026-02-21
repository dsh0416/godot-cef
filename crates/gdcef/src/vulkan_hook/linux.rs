//! Linux-specific Vulkan hook implementation.
//!
//! On Linux, we need to inject `VK_KHR_external_memory_fd` and `VK_EXT_external_memory_dma_buf`
//! to enable sharing textures via DMA-BUF file descriptors between Godot and CEF.

use std::ffi::CStr;

use super::common::define_vulkan_hook;

const VK_KHR_EXTERNAL_MEMORY_NAME: &CStr = c"VK_KHR_external_memory";
const VK_KHR_EXTERNAL_MEMORY_FD_NAME: &CStr = c"VK_KHR_external_memory_fd";
const VK_EXT_EXTERNAL_MEMORY_DMA_BUF_NAME: &CStr = c"VK_EXT_external_memory_dma_buf";

define_vulkan_hook!(
    log_prefix: "[VulkanHook/Linux]",
    vulkan_lib: "libvulkan.so.1",
    status_extension: VK_EXT_EXTERNAL_MEMORY_DMA_BUF_NAME,
    required_extensions: [
        VK_KHR_EXTERNAL_MEMORY_NAME,
        VK_KHR_EXTERNAL_MEMORY_FD_NAME,
        VK_EXT_EXTERNAL_MEMORY_DMA_BUF_NAME
    ]
);
