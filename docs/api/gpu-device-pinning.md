# GPU Device Pinning

This page explains how Godot CEF ensures CEF uses the same GPU as Godot on multi-GPU systems, enabling successful texture sharing for accelerated rendering.

## The Multi-GPU Problem

Modern systems often have multiple GPUs:

- **Laptops** — Integrated GPU (Intel/AMD) + Discrete GPU (NVIDIA/AMD)
- **Desktops** — Multiple discrete GPUs for multi-monitor setups
- **Workstations** — Professional GPUs alongside consumer GPUs

When sharing textures between processes (Godot and CEF's renderer), both must use the **same physical GPU**. Cross-GPU texture sharing is not supported by the underlying APIs.

### What Happens Without Device Pinning

Without explicit GPU selection:
1. Godot selects a GPU (typically the discrete GPU for better performance)
2. CEF's renderer subprocess independently selects a GPU (often defaulting to index 0, the integrated GPU)
3. Godot exports a texture handle from GPU A
4. CEF tries to import it on GPU B
5. **Import fails** — the handle is invalid on a different device

This results in black textures or rendering failures.

## The Solution: Command-Line GPU Selection

Godot CEF uses Chromium's `--gpu-vendor-id` and `--gpu-device-id` command-line switches to specify which GPU CEF should use. This approach works across all platforms without requiring hooks or environment variable manipulation.

### How It Works

```
┌─────────────────────────────────────────────────────────────────┐
│                        Godot Process                            │
│                                                                 │
│  1. Query RenderingDevice for GPU vendor/device IDs             │
│     - Windows D3D12: DXGI adapter description                   │
│     - Windows/Linux Vulkan: VkPhysicalDeviceProperties          │
│     - macOS Metal: IOKit registry properties                    │
│                                                                 │
│  2. Pass IDs to CEF subprocesses via command-line switches      │
│     --gpu-vendor-id=4318 --gpu-device-id=7815                   │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                     CEF Subprocess                              │
│                                                                 │
│  Chromium's GPU process uses the vendor/device IDs to select    │
│  the matching GPU adapter for rendering                         │
└─────────────────────────────────────────────────────────────────┘
```

### Platform-Specific GPU ID Retrieval

| Platform | Backend | Method |
|----------|---------|--------|
| Windows | D3D12 | Query `IDXGIAdapter::GetDesc()` for `VendorId` and `DeviceId` |
| Windows | Vulkan | Query `VkPhysicalDeviceProperties` via `vkGetPhysicalDeviceProperties2` |
| Linux | Vulkan | Query `VkPhysicalDeviceProperties` via `vkGetPhysicalDeviceProperties2` |
| macOS | Metal | Query IOKit registry for `vendor-id` and `device-id` properties |

### Code Flow

**Step 1:** During CEF initialization, Godot CEF queries the GPU IDs:

```rust
// In gdcef/src/cef_init.rs
use crate::accelerated_osr::get_godot_gpu_device_ids;
if let Some((vendor_id, device_id)) = get_godot_gpu_device_ids() {
    osr_app = osr_app.with_gpu_device_ids(vendor_id, device_id);
}
```

**Step 2:** The IDs are passed to CEF subprocesses in `on_before_child_process_launch`:

```rust
// In cef_app/src/lib.rs
if let Some(ids) = &self.handler.gpu_device_ids {
    command_line.append_switch_with_value(
        Some(&"gpu-vendor-id".into()),
        Some(&ids.to_vendor_arg().as_str().into()),  // e.g., "4318" (decimal)
    );
    command_line.append_switch_with_value(
        Some(&"gpu-device-id".into()),
        Some(&ids.to_device_arg().as_str().into()),  // e.g., "7815" (decimal)
    );
}
```

## Platform Availability

| Platform | GPU Pinning | Status |
|----------|-------------|--------|
| Windows (D3D12) | Command-line switches | ✅ Supported |
| Windows (Vulkan) | Command-line switches | ✅ Supported |
| Linux (Vulkan) | Command-line switches | ✅ Supported |
| macOS (Metal) | Command-line switches | ✅ Supported |

### macOS Notes

On Apple Silicon (M-series chips), the `vendor-id` and `device-id` properties do not exist in the IOKit registry because the GPU is integrated into the SoC rather than being a discrete PCI device. In this case, GPU device pinning is simply skipped. This is fine since Apple Silicon Macs have only one GPU — both Godot and CEF will always use the same GPU without explicit pinning.

## Debugging GPU Pinning

### Diagnostic Output

Godot CEF prints GPU information during initialization:

```
[AcceleratedOSR/D3D12] Godot GPU: vendor=0x10de, device=0x1e87, name=NVIDIA GeForce RTX 3080
[CefInit] Godot GPU: vendor=0x10de, device=0x1e87 - will pass to CEF subprocesses
```

### Common Issues

**Black textures**
- Verify both Godot and CEF report the same GPU in logs
- Check that external memory extensions are enabled (see [Vulkan Support](./vulkan-support.md))
- On multi-GPU systems, ensure the correct GPU is being selected

**GPU ID retrieval failures**
- Check that Godot is using a supported rendering backend (D3D12, Vulkan, or Metal)
- Verify graphics drivers are up to date

### Verifying GPU Selection

To confirm CEF is using the correct GPU:

1. Enable CEF remote debugging (`remote_debugging_port` property)
2. Open Chrome DevTools (`chrome://inspect`)
3. Navigate to `chrome://gpu` in the CEF browser
4. Check "Graphics Feature Status" for the active GPU

## Common GPU Vendor IDs

| Vendor | ID |
|--------|-----|
| NVIDIA | `0x10de` |
| AMD | `0x1002` |
| Intel | `0x8086` |
| Apple | `0x106b` |

## Advantages Over Previous Approach

The command-line switch approach has several advantages over the previous hook-based implementation:

1. **Simpler architecture** — No function hooking or vtable patching required
2. **Cross-platform** — Same mechanism works on Windows, Linux, and macOS
3. **More reliable** — No timing issues with hook installation
4. **Antivirus friendly** — No memory manipulation that might trigger security software

## See Also

- [Vulkan Support](./vulkan-support.md) — External memory extension injection
- [Properties](./properties.md) — `enable_accelerated_osr` configuration
