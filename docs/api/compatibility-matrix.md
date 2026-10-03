# Compatibility Matrix

This matrix lists supported platform/backend combinations for GPU-accelerated OSR.

This page covers architectures in the full GitHub Release addon. The smaller
Asset Store addon omits Windows/Linux ARM64; use the full package for those
targets. See [Distribution variants](./distribution-variants).

## Version Baseline

Godot 4.6 or later is required.

The Rust `cef` / `cef-dll-sys` crates in `Cargo.lock` define the CEF runtime version through their build metadata. After installing and activating the project toolchain as described in [Development Setup](https://github.com/dsh0416/godot-cef/blob/main/CONTRIBUTING.md#development-setup), derive that version from the repository root when installing CEF binaries manually:

```bash
export CEF_PATH="$HOME/.local/share/cef"
CEF_VERSION="$(cargo run --locked --quiet -p xtask -- cef-version)" || exit 1
export-cef-dir --version "$CEF_VERSION" --force "$CEF_PATH"
```

This keeps the downloaded runtime files aligned with the Rust bindings.

## Supported GPU Backends

| Platform | Architecture | Godot Backend | Integration |
|----------|--------------|---------------|-------------|
| Windows | x86_64, ARM64 | Direct3D12 | Native shared textures |
| Windows | x86_64 | Vulkan | Extension and queue hooks |
| macOS | x86_64, ARM64 | Metal | Native IOSurface sharing |
| Linux | x86_64 | Vulkan | Extension and queue hooks |

## Fallback Conditions

Even on a supported backend, Godot CEF falls back to software rendering when:

- `enable_accelerated_osr` is disabled on `CefTexture`.
- Platform texture importer creation fails.
- Required Vulkan external memory extensions cannot be injected or are unavailable.

These conditions apply during browser initialization. After accelerated rendering
starts, capture or publication errors retain the last successfully published frame.
See [Accelerated frame handoff](./accelerated-handoff) for GPU completion and
resource lifetime details.

## Diagnostics

At startup, Godot CEF logs:

- Detected backend and whether accelerated OSR is supported.
- Fallback reason when accelerated rendering cannot be used.

During browser creation, logs also indicate whether each `CefTexture` instance starts in accelerated or software mode.
