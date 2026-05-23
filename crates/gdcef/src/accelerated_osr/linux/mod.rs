//! Linux-specific accelerated OSR implementation.
//!
//! On Linux, we use Vulkan with DMA-BUF external memory extensions to import
//! shared textures from CEF's compositor process.

mod vulkan;

use super::RenderBackend;
use cef::AcceleratedPaintInfo;
use godot::global::{godot_print, godot_warn};
use godot::prelude::*;
use std::fs;
use std::path::Path;

const NVIDIA_VENDOR_ID: u32 = 0x10de;

pub fn get_godot_gpu_device_ids() -> Option<(u32, u32)> {
    vulkan::get_godot_gpu_device_ids()
}

pub struct GodotTextureImporter {
    vulkan_importer: vulkan::VulkanTextureImporter,
}

impl GodotTextureImporter {
    pub fn new() -> Option<Self> {
        let render_backend = RenderBackend::detect();

        if !render_backend.supports_accelerated_osr() {
            godot_warn!(
                "[AcceleratedOSR/Linux] Render backend {:?} does not support accelerated OSR",
                render_backend
            );
            return None;
        }

        match render_backend {
            RenderBackend::Vulkan => {
                let (supported, reason) = vulkan_support_diagnostic();
                if !supported {
                    godot_warn!(
                        "[AcceleratedOSR/Linux] Vulkan accelerated OSR unavailable: {}",
                        reason
                    );
                    return None;
                }

                let vulkan_importer = vulkan::VulkanTextureImporter::new()?;
                godot_print!("[AcceleratedOSR/Linux] Using Vulkan backend with DMA-BUF");
                Some(Self { vulkan_importer })
            }
            _ => {
                godot_warn!(
                    "[AcceleratedOSR/Linux] Unsupported render backend: {:?}",
                    render_backend
                );
                None
            }
        }
    }

    pub fn queue_copy(&mut self, info: &AcceleratedPaintInfo) -> Result<(), String> {
        self.vulkan_importer.queue_copy(info)
    }

    pub fn process_pending_copy(&mut self, dst_rd_rid: Rid) -> Result<(), String> {
        self.vulkan_importer.process_pending_copy(dst_rd_rid)
    }

    pub fn wait_for_copy(&mut self) -> Result<(), String> {
        self.vulkan_importer.wait_for_copy()
    }
}

pub fn is_supported() -> bool {
    let render_backend = RenderBackend::detect();
    if !render_backend.supports_accelerated_osr() {
        return false;
    }

    match render_backend {
        RenderBackend::Vulkan => vulkan_support_diagnostic().0,
        _ => false,
    }
}

unsafe impl Send for GodotTextureImporter {}
unsafe impl Sync for GodotTextureImporter {}

pub fn vulkan_support_diagnostic() -> (bool, String) {
    match vulkan_support_probe() {
        Ok(reason) => (true, reason),
        Err(reason) => (false, reason),
    }
}

fn vulkan_support_probe() -> Result<String, String> {
    let Some((vendor_id, device_id)) = vulkan::get_godot_gpu_device_ids() else {
        return Ok(
            "Vulkan backend supports accelerated OSR; GPU vendor could not be determined"
                .to_string(),
        );
    };

    if vendor_id != NVIDIA_VENDOR_ID {
        return Ok(format!(
            "Vulkan backend on GPU vendor 0x{vendor_id:04x} supports accelerated OSR"
        ));
    }

    match nvidia_kernel_driver(device_id) {
        Some(driver) if driver == "nouveau" => {
            Ok("NVIDIA Vulkan backend uses nouveau, which supports accelerated OSR".to_string())
        }
        Some(driver) if driver == "nvidia" => nvidia_drm_modeset_support_reason(),
        Some(driver) => Err(format!(
            "NVIDIA Vulkan backend uses unsupported kernel driver `{driver}`"
        )),
        None => {
            if nvidia_drm_modeset_enabled() {
                Ok("NVIDIA Vulkan backend has nvidia-drm.modeset enabled".to_string())
            } else {
                Err(
                    "NVIDIA Vulkan backend could not verify nouveau usage, and nvidia-drm.modeset is not enabled"
                        .to_string(),
                )
            }
        }
    }
}

fn nvidia_drm_modeset_support_reason() -> Result<String, String> {
    if nvidia_drm_modeset_enabled() {
        Ok(
            "NVIDIA Vulkan backend uses nvidia kernel modules with nvidia-drm.modeset enabled"
                .to_string(),
        )
    } else {
        Err(
            "NVIDIA Vulkan backend using proprietary/open kernel modules requires nvidia-drm.modeset=1"
                .to_string(),
        )
    }
}

fn nvidia_kernel_driver(device_id: u32) -> Option<String> {
    kernel_driver_for_pci_device(
        Path::new("/sys/bus/pci/devices"),
        NVIDIA_VENDOR_ID,
        device_id,
    )
}

fn kernel_driver_for_pci_device(
    sysfs_pci_devices: &Path,
    vendor_id: u32,
    device_id: u32,
) -> Option<String> {
    let entries = fs::read_dir(sysfs_pci_devices).ok()?;

    for entry in entries.flatten() {
        let device_path = entry.path();
        let Some(entry_vendor_id) = read_sysfs_hex_id(&device_path.join("vendor")) else {
            continue;
        };
        if entry_vendor_id != vendor_id {
            continue;
        }

        let Some(entry_device_id) = read_sysfs_hex_id(&device_path.join("device")) else {
            continue;
        };
        if entry_device_id != device_id {
            continue;
        }

        let Some(driver_name) = driver_name_from_device_path(&device_path) else {
            continue;
        };

        return Some(driver_name);
    }

    None
}

fn read_sysfs_hex_id(path: &Path) -> Option<u32> {
    let value = fs::read_to_string(path).ok()?;
    parse_hex_id(value.trim())
}

fn parse_hex_id(value: &str) -> Option<u32> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    u32::from_str_radix(value, 16).ok()
}

fn driver_name_from_device_path(device_path: &Path) -> Option<String> {
    let driver_path = fs::read_link(device_path.join("driver")).ok()?;
    driver_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
}

fn nvidia_drm_modeset_enabled() -> bool {
    fs::read_to_string("/sys/module/nvidia_drm/parameters/modeset")
        .ok()
        .and_then(|value| parse_kernel_bool(value.trim()))
        .unwrap_or(false)
}

fn parse_kernel_bool(value: &str) -> Option<bool> {
    match value {
        "1" | "Y" | "y" | "true" | "True" => Some(true),
        "0" | "N" | "n" | "false" | "False" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sysfs_hex_ids() {
        assert_eq!(parse_hex_id("0x10de"), Some(NVIDIA_VENDOR_ID));
        assert_eq!(parse_hex_id("10de"), Some(NVIDIA_VENDOR_ID));
        assert_eq!(parse_hex_id("not-hex"), None);
    }

    #[test]
    fn parses_kernel_bool_values() {
        assert_eq!(parse_kernel_bool("Y"), Some(true));
        assert_eq!(parse_kernel_bool("1"), Some(true));
        assert_eq!(parse_kernel_bool("N"), Some(false));
        assert_eq!(parse_kernel_bool("0"), Some(false));
        assert_eq!(parse_kernel_bool("maybe"), None);
    }
}
