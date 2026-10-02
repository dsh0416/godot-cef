# Compatibility Matrix

This matrix summarizes the expected rendering mode behavior for each platform/backend combination.

This page covers architectures in the full GitHub Release addon. The smaller
Asset Store addon omits Windows/Linux ARM64; use the full package for those
targets. See [Distribution variants](./distribution-variants).

## Version Baseline

The Rust `cef` / `cef-dll-sys` crates in `Cargo.lock` define the CEF runtime version through their build metadata. After installing and activating the project toolchain as described in [Development Setup](https://github.com/dsh0416/godot-cef/blob/main/CONTRIBUTING.md#development-setup), derive that version from the repository root when installing CEF binaries manually:

```bash
export CEF_PATH="$HOME/.local/share/cef"
CEF_VERSION="$(cargo run --locked --quiet -p xtask -- cef-version)" || exit 1
export-cef-dir --version "$CEF_VERSION" --force "$CEF_PATH"
```

This keeps the downloaded runtime files aligned with the Rust bindings.

## Runtime Rendering Matrix

| Platform | Architecture | Godot Backend | Accelerated OSR | Default Outcome |
|----------|--------------|---------------|-----------------|-----------------|
| Windows  | x86_64       | Direct3D12    | Yes             | Accelerated |
| Windows  | x86_64       | Vulkan        | Yes (hook-based) | Accelerated |
| Windows  | ARM64        | Direct3D12    | Yes             | Accelerated |
| Windows  | any          | OpenGL        | No              | Software fallback |
| Windows  | ARM64        | Vulkan        | No (hooks unsupported) | Software fallback |
| macOS    | any          | Metal         | Yes             | Accelerated |
| macOS    | any          | Vulkan        | No              | Software fallback |
| macOS    | any          | OpenGL        | No              | Software fallback |
| Linux    | x86_64       | Vulkan        | Yes (hook-based) | Accelerated |
| Linux    | any          | OpenGL        | No              | Software fallback |
| Linux    | ARM64        | Vulkan        | No (hooks unsupported) | Software fallback |

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
