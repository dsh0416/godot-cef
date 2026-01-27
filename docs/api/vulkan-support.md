# Vulkan Support

This page documents how Godot CEF enables GPU-accelerated rendering on Vulkan backends through runtime function hooking, and the limitations of this approach.

## Background

GPU-accelerated offscreen rendering (OSR) in CEF requires sharing textures between the CEF renderer process and the host application (Godot). This is achieved through platform-specific external memory APIs:

| Platform | Graphics API | Sharing Mechanism |
|----------|--------------|-------------------|
| Windows  | DirectX 12   | NT Handles (native support) |
| Windows  | Vulkan       | `VK_KHR_external_memory_win32` |
| macOS    | Vulkan       | `VK_EXT_metal_objects` |
| macOS    | Metal        | IOSurface (native support) |
| Linux    | Vulkan       | `VK_EXT_external_memory_dma_buf` `VK_KHR_external_memory_fd` |

The problem is that **Godot does not enable these Vulkan external memory extensions by default** when creating its Vulkan device. Without these extensions, texture sharing between CEF and Godot is impossible.

## The Hook Solution

Since Godot doesn't provide an API to request additional Vulkan extensions during device creation, Godot CEF uses **runtime function hooking** to inject the required extensions.

### How It Works

1. During GDExtension initialization (at the `Core` stage, before `RenderingServer` is created), we install a hook on `vkCreateDevice`
2. When Godot calls `vkCreateDevice` to create its Vulkan device, our hook intercepts the call
3. The hook modifies the `VkDeviceCreateInfo` structure to add the required external memory extensions
4. The modified request is passed to the real `vkCreateDevice` function
5. Godot now has a Vulkan device with external memory support enabled

### Platform-Specific Extensions

**Windows:**
- `VK_KHR_external_memory` — Base extension for external memory
- `VK_KHR_external_memory_win32` — Windows-specific HANDLE sharing

**macOS:**
- `VK_KHR_external_memory` — Base extension for external memory
- `VK_EXT_metal_objects` — Metal objects sharing

**Linux:**
- `VK_KHR_external_memory` — Base extension for external memory
- `VK_KHR_external_memory_fd` — File descriptor based sharing
- `VK_EXT_external_memory_dma_buf` — DMA-BUF sharing for zero-copy transfers

## Multi-GPU Support

On systems with multiple GPUs (e.g., laptops with integrated + discrete graphics), **CEF must use the same GPU as Godot** for texture sharing to work. This is handled via command-line switches (`--gpu-vendor-id` and `--gpu-device-id`) passed to CEF subprocesses.

::: tip
For detailed information about how GPU device pinning works, see [GPU Device Pinning](./gpu-device-pinning.md).
:::

## Limitations

### Architecture Requirement (x86_64 Only)

::: warning
Vulkan hook-based acceleration is **only available on x86_64 (64-bit x86) architectures**.
:::

The hooking mechanism relies on the [retour](https://github.com/darfink/retour-rs) library for runtime function detouring. This library currently does not support ARM64 architecture, which means:

- **Windows ARM64** — Vulkan hooks not available
- **Linux ARM64** — Vulkan hooks not available  
- **macOS (Apple Silicon)** — Vulkan hooks not available

On unsupported architectures, the extension automatically falls back to software rendering.

### macOS Vulkan Not Supported

macOS Vulkan support (via MoltenVK) does not benefit from the hook mechanism due to fundamental technical limitations:

1. **Static Linking** — Godot statically links MoltenVK into its binary. This means `vkCreateDevice` calls go directly to embedded code rather than through a dynamic library's PLT/GOT (Procedure Linkage Table / Global Offset Table). Function hooking libraries like retour work by intercepting calls at these indirection points, which don't exist with static linking. Even with ARM64 support in retour, there's no viable hook target.

2. **Native Metal Alternative** — macOS already has native Metal support which provides better performance and doesn't require any hooks. Metal's IOSurface sharing mechanism works natively without extension injection.

3. **Limited Benefit** — MoltenVK is a compatibility layer translating Vulkan to Metal. Using Vulkan on macOS adds overhead compared to using Metal directly.

::: tip
Use the **Metal backend** on macOS for GPU-accelerated rendering. It's the native API and works out of the box on both Intel and Apple Silicon Macs.
:::

### Timing Sensitivity

The hook must be installed **before** Godot creates its Vulkan device. This is why installation happens during the `Core` initialization stage of GDExtension. If the hook is installed too late, the Vulkan device will be created without the required extensions.

### Stability Considerations

Function hooking is inherently fragile:

- Updates to Vulkan drivers could potentially change behavior
- Antivirus software may flag hook-based modifications
- Some Vulkan layers or debugging tools might interfere with hooks

If you experience issues with accelerated rendering, try:
1. Updating your graphics drivers
2. Disabling Vulkan validation layers during normal use
3. Falling back to software rendering by setting `enable_accelerated_osr = false`

## Platform Support Summary

| Platform | Architecture | Vulkan Accelerated OSR | Notes |
|----------|--------------|------------------------|-------|
| Windows  | x86_64       | ✅ Supported           | Via `vkCreateDevice` extension injection hook |
| Windows  | ARM64        | ❌ Not supported       | retour doesn't support ARM64 |
| Linux    | x86_64       | ✅ Supported           | Via `vkCreateDevice` extension injection hook |
| Linux    | ARM64        | ❌ Not supported       | retour doesn't support ARM64 |
| macOS    | Any          | ❌ Not applicable      | Static linking of MoltenVK prevents hooking; use Metal backend |

## Future: Proper Godot API

This hook-based approach is a workaround. The proper solution would be for Godot to provide an API allowing GDExtensions to request additional Vulkan extensions during device creation.

A proposal for this feature exists: [godotengine/godot-proposals#13969](https://github.com/godotengine/godot-proposals/issues/13969)

Once this proposal is implemented, Godot CEF can migrate away from the hook-based approach to a cleaner, officially supported method.

## Debugging

When hooks are installed, diagnostic messages are printed to stderr:

```
[VulkanHook/Windows] Installing vkCreateDevice hook...
[VulkanHook/Windows] Hook installed successfully
[VulkanHook/Windows] Injecting external memory extensions
[VulkanHook/Windows] Adding VK_KHR_external_memory
[VulkanHook/Windows] Adding VK_KHR_external_memory_win32
[VulkanHook/Windows] Successfully created device with external memory extensions
```

On Linux:
```
[VulkanHook/Linux] Installing vkCreateDevice hook...
[VulkanHook/Linux] Hook installed successfully
[VulkanHook/Linux] Injecting external memory extensions
[VulkanHook/Linux] Adding VK_KHR_external_memory
[VulkanHook/Linux] Adding VK_KHR_external_memory_fd
[VulkanHook/Linux] Adding VK_EXT_external_memory_dma_buf
[VulkanHook/Linux] Successfully created device with external memory extensions
```

If you see messages about extensions not being supported or hook installation failures, accelerated rendering will fall back to software mode.

## See Also

- [GPU Device Pinning](./gpu-device-pinning.md) — Multi-GPU support via command-line switches
- [Properties](./properties.md) — `enable_accelerated_osr` property documentation
- [GitHub Issue #4](https://github.com/dsh0416/godot-cef/issues/4) — Tracking issue for Vulkan support

