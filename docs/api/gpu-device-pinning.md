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

## The Solution: DXGI Adapter Filtering

On Windows, Godot CEF uses a **DXGI hook** to force CEF to use the same GPU adapter as Godot. This works for both DirectX 12 and Vulkan backends (since Vulkan on Windows uses DXGI for adapter enumeration).

### How GPU Identification Works

Each GPU adapter has a **LUID (Locally Unique Identifier)** — a 64-bit value that uniquely identifies the adapter within the current boot session.

```
LUID Structure:
┌─────────────────┬─────────────────┐
│   HighPart (32) │   LowPart (32)  │
└─────────────────┴─────────────────┘
```

**Step 1:** When Godot CEF initializes, it queries Godot's `RenderingDevice` for the adapter LUID:

```
Godot RenderingDevice → get_driver_resource(DRIVER_RESOURCE_VULKAN_PHYSICAL_DEVICE)
                      → Query VkPhysicalDeviceIDProperties
                      → Extract deviceLUID
```

**Step 2:** This LUID is passed to the CEF helper subprocess via command-line arguments.

**Step 3:** The helper subprocess installs DXGI hooks before CEF initializes.

### DXGI Hook Architecture

The hook intercepts DXGI factory creation to control adapter enumeration:

```
Application calls CreateDXGIFactory1/2
           │
           ▼
    ┌──────────────┐
    │  Our Hook    │ ◄── Intercepts the call
    └──────┬───────┘
           │
           ▼
    ┌──────────────┐
    │ Real Factory │ ◄── Factory is created normally
    └──────┬───────┘
           │
           ▼
    ┌──────────────┐
    │ Patch VTable │ ◄── EnumAdapters methods redirected
    └──────┬───────┘
           │
           ▼
    Factory returned to caller (with patched vtable)
```

### VTable Patching

COM interfaces like `IDXGIFactory` use virtual function tables (vtables). We patch specific methods:

| VTable Index | Method | Our Hook Action |
|--------------|--------|-----------------|
| 7 | `EnumAdapters` | Redirect to our filter |
| 12 | `EnumAdapters1` | Redirect to our filter |

The patched methods implement adapter filtering:

```
Original Behavior:
  EnumAdapters(0) → Adapter A (integrated)
  EnumAdapters(1) → Adapter B (discrete)  ← Target
  EnumAdapters(2) → DXGI_ERROR_NOT_FOUND

Hooked Behavior:
  EnumAdapters(0) → Adapter B (discrete)  ← Only target visible
  EnumAdapters(1) → DXGI_ERROR_NOT_FOUND
```

### Adapter Selection Logic

When a factory is created, our hook:

1. **Enumerates all adapters** using the original (unhooked) function
2. **Matches by LUID** to find the target adapter's index
3. **Stores the target index** for use by the filter
4. **Patches the vtable** to redirect enumeration calls

The filter then:
- Returns the **target adapter** when index 0 is requested
- Returns `DXGI_ERROR_NOT_FOUND` for all other indices

This makes CEF "see" only one adapter — the same one Godot is using.

## Platform Availability

| Platform | GPU Pinning Method | Status |
|----------|-------------------|--------|
| Windows (D3D12) | DXGI Hook | ✅ Supported |
| Windows (Vulkan) | DXGI Hook | ✅ Supported |
| Linux (Vulkan) | Device UUID matching | ✅ Supported |
| macOS (Metal) | Not needed | ✅ Single GPU selection by Metal |

### Linux: Device UUID Matching

On Linux, GPU identification uses **Device UUIDs** from Vulkan's `VkPhysicalDeviceIDProperties`. The UUID is passed to the CEF helper, which uses it to select the matching GPU when initializing its Vulkan device.

### macOS: Metal Handles It

macOS with Metal doesn't require explicit device pinning because:
- IOSurface handles work across the unified memory architecture
- Metal device selection is handled by the system

## Debugging GPU Pinning

### Diagnostic Output

The DXGI hook prints diagnostic information to stderr:

```
[DXGI Hook] Installing hooks for adapter LUID: 0, 12345
[DXGI Hook] Adapter 0: LUID (0, 11111), Name: Intel UHD Graphics
[DXGI Hook] Adapter 1: LUID (0, 12345), Name: NVIDIA GeForce RTX 3080
[DXGI Hook] Target adapter found at index 1 (LUID: 0, 12345)
[DXGI Hook] Vtable patched - only adapter 1 visible at index 0, others hidden
```

### Common Issues

**"No adapter found matching LUID"**
- The target GPU may have been disabled or removed
- Driver update changed the LUID
- Try restarting the application

**Hook installation failures**
- Antivirus software may block function hooking
- Try adding an exception for the helper executable

**Black textures despite successful hooks**
- Verify both Godot and CEF report the same GPU in logs
- Check that external memory extensions are enabled (see [Vulkan Support](./vulkan-support.md))

### Verifying GPU Selection

To confirm CEF is using the correct GPU:

1. Enable CEF remote debugging (`remote_debugging_port` property)
2. Open Chrome DevTools (`chrome://inspect`)
3. Navigate to `chrome://gpu` in the CEF browser
4. Check "Graphics Feature Status" for the active GPU

## Technical Details

### Memory Protection

The vtable resides in read-only memory. The hook temporarily changes memory protection:

```
VirtualProtect(vtable_slot, PAGE_EXECUTE_READWRITE)
    → Write hook pointer
VirtualProtect(vtable_slot, original_protection)
```

### Thread Safety

A mutex protects vtable patching to prevent race conditions when multiple factories are created simultaneously.

### Original Function Preservation

Original function pointers are stored atomically:
- `ORIGINAL_ENUM_ADAPTERS` — For `IDXGIFactory::EnumAdapters`
- `ORIGINAL_ENUM_ADAPTERS1` — For `IDXGIFactory1::EnumAdapters1`

This ensures the filter can call the real enumeration functions to access the target adapter.

## Limitations

### First-Factory Timing

The hook must intercept the **first** DXGI factory creation. If CEF creates a factory before hooks are installed, GPU pinning fails.

The helper subprocess installs hooks immediately on startup, before any CEF initialization, to ensure this timing requirement is met.

### Single Target GPU

Only one target GPU can be specified per process. Multi-GPU rendering (e.g., SLI/CrossFire) is not supported.

### Session-Specific LUIDs

LUIDs are valid only for the current Windows session. They may change after:
- System restart
- Driver updates
- Hardware changes

The LUID is queried fresh each time the application starts, so this is handled automatically.

## See Also

- [Vulkan Support](./vulkan-support.md) — External memory extension injection
- [Properties](./properties.md) — `enable_accelerated_osr` configuration

