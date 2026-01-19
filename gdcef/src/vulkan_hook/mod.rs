//! Vulkan device creation hook for injecting external memory extensions.
//!
//! This module hooks `vkCreateDevice` during GDExtension initialization (at the Core stage)
//! to inject platform-specific external memory extensions that Godot doesn't enable by default.
//!
//! Platform-specific extensions:
//! - Windows: `VK_KHR_external_memory_win32` for HANDLE sharing
//! - Linux: `VK_EXT_external_memory_dma_buf` for DMA-Buf sharing
//! - macOS: `VK_EXT_metal_objects` for IOSurface sharing (not yet supported - retour lacks ARM64)

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "windows")]
pub use windows::install_vulkan_hook;

#[cfg(target_os = "linux")]
pub use linux::install_vulkan_hook;

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub fn install_vulkan_hook() {
    // No-op on other platforms for now
    // macOS: retour doesn't support ARM64
}
