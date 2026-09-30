# Compatibility Matrix

This matrix summarizes the expected rendering mode behavior for each platform/backend combination.

Official binary targets: Windows x86_64, Linux x86_64, and macOS universal
(x86_64/ARM64). **Breaking change:** official Windows ARM64 and Linux ARM64 builds,
packages, and Godot export registrations have been removed. Users can remain on
an earlier release or follow the [unsupported source-build guide](./unsupported-targets),
register their own manifest, and validate their results. Windows x86_64 emulation
is unverified. This distribution change may require a major release; see
[#238](https://github.com/dsh0416/godot-cef/issues/238).

## Version Baseline

Current builds are based on the Rust `cef` / `cef-dll-sys` crates resolved as `152.3.0+152.0.6` in `Cargo.lock`. The matching CEF runtime version is pinned as `CEF_VERSION` in `mise.toml`; use it when installing CEF binaries manually:

```bash
export CEF_PATH="$HOME/.local/share/cef"
export-cef-dir --version "$CEF_VERSION" --force "$CEF_PATH"
```

This keeps the downloaded runtime files aligned with the Rust bindings.

## Runtime Rendering Matrix

| Platform | Architecture | Godot Backend | Accelerated OSR | Default Outcome |
|----------|--------------|---------------|-----------------|-----------------|
| Windows  | x86_64       | Direct3D12    | Yes             | Accelerated |
| Windows  | x86_64       | Vulkan        | Yes (hook-based) | Accelerated |
| Windows  | x86_64          | OpenGL        | No              | Software fallback |
| macOS    | any          | Metal         | Yes             | Accelerated |
| macOS    | any          | Vulkan        | No              | Software fallback |
| macOS    | any          | OpenGL        | No              | Software fallback |
| Linux    | x86_64       | Vulkan        | Yes (hook-based) | Accelerated |
| Linux    | x86_64          | OpenGL        | No              | Software fallback |

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
