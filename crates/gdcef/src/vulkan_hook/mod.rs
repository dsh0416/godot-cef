//! Vulkan device creation hook for injecting external memory extensions.
//!
//! This module hooks `vkCreateDevice` during GDExtension initialization (at the Core stage)
//! to inject platform-specific external memory extensions that Godot doesn't enable by default.
//!
//! Platform-specific extensions:
//! - Windows: `VK_KHR_external_memory_win32` for HANDLE sharing
//! - Linux: `VK_EXT_external_memory_dma_buf` for DMA-Buf sharing
//! - macOS: Not supported — Godot statically links MoltenVK, making hook injection impossible. Use the Metal backend instead, which supports IOSurface sharing natively.

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
mod windows;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod linux;

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
pub use windows::install_vulkan_hook;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use linux::install_vulkan_hook;

#[cfg(not(any(
    all(target_os = "windows", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "x86_64")
)))]
pub fn install_vulkan_hook() {
    // No-op on unsupported platforms:
    // - ARM64: retour doesn't support ARM64 architecture
    // - macOS: Godot statically links MoltenVK, so there's no dynamic symbol to hook
    //          (even if retour supported ARM64, hooking wouldn't work on macOS)
}
