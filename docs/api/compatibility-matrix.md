# Compatibility Matrix

This matrix summarizes the expected rendering mode behavior for each platform/backend combination.

## Runtime Rendering Matrix

| Platform | Architecture | Godot Backend | Accelerated OSR | Default Outcome |
|----------|--------------|---------------|-----------------|-----------------|
| Windows  | x86_64       | Direct3D12    | Yes             | Accelerated |
| Windows  | x86_64       | Vulkan        | Yes (hook-based) | Accelerated |
| Windows  | any          | OpenGL        | No              | Software fallback |
| Windows  | ARM64        | Vulkan        | No              | Software fallback |
| macOS    | any          | Metal         | Yes             | Accelerated |
| macOS    | any          | Vulkan        | No              | Software fallback |
| macOS    | any          | OpenGL        | No              | Software fallback |
| Linux    | x86_64       | Vulkan        | Yes (hook-based) | Accelerated |
| Linux    | any          | OpenGL        | No              | Software fallback |
| Linux    | ARM64        | Vulkan        | No              | Software fallback |

## Fallback Conditions

Even on a supported backend, Godot CEF falls back to software rendering when:

- `enable_accelerated_osr` is disabled on `CefTexture`.
- Platform texture importer creation fails.
- Required Vulkan external memory extensions cannot be injected or are unavailable.

## Diagnostics

At startup, Godot CEF logs:

- Detected backend and whether accelerated OSR is supported.
- Fallback reason when accelerated rendering cannot be used.

During browser creation, logs also indicate whether each `CefTexture` instance starts in accelerated or software mode.

