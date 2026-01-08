# Godot CEF

A high-performance Chromium Embedded Framework (CEF) integration for Godot Engine 4.5 and above, written in Rust. Render web content directly inside your Godot games and applications with full support for modern web standards, JavaScript, HTML5, and CSS3.

## ✨ Features

- **Web Rendering in Godot** — Display any web content as a texture using the `CefTexture` node (extends `TextureRect`)
- **Accelerated Off-Screen Rendering** — GPU-accelerated rendering using platform-native graphics APIs for maximum performance
- **Software Rendering Fallback** — Automatic fallback to CPU-based rendering when accelerated rendering is unavailable
- **Dynamic Scaling** — Automatic handling of DPI changes and window resizing
- **Multi-Process Architecture** — Proper CEF subprocess handling for stability and consistency
- **Remote Debugging** — Built-in Chrome DevTools support

## 📋 Platform Support Matrix

| Platform | DirectX 12 | Metal | Vulkan | Software Rendering |
|----------|---------------|-----------------|-------------------|--------|
| **Windows** | ✅ (Note 1) | n.a. | ❌ (Note 2)| ✅ |
| **macOS** | n.a. | ✅ | ❌ (Note 3) | ✅ |
| **Linux** | n.a. | n.a. | ❌ (Note 4) | ✅ |

### Note
1. For Windows DirectX 12 backend, it requires at least Godot 4.6 beta 2 to work. Since Godot 4.5.1 contains a bug when calling `RenderingDevice.get_driver_resource` on DirectX 12 textures ALWAYS returns 0.
2. Vulkan on Windows requires `VK_KHR_external_memory_win32` to import Windows Handle into VKImage.  Godot's vulkan device doesn't start with such extensions enabled.
3. Vulkan on macOS requires `VK_EXT_metal_objects` to import IOSurface into VKImage. Godot's vulkan device doesn't start with such extensions enabled.
4. Vulkan on Linux requires `VK_EXT_external_memory_dma_buf` to import DMABuf into VKImage. Godot's vulkan device doesn't start with such extensions enabled.
5. On platforms where accelerated rendering is not yet implemented, the extension automatically falls back to software rendering using CPU-based frame buffers.

## 🛠️ Prerequisites

- **Rust** (1.92+) — Install via [rustup](https://rustup.rs/)
- **Godot** (4.5+) — Download from [godotengine.org](https://godotengine.org/)
- **CEF Binaries** — Automatically downloaded during build

## 📦 Building

### Step 1: Install the CEF Export Tool

```bash
cargo install export-cef-dir
```

This tool downloads and extracts the correct CEF binaries for your platform. For cross-platform building, download from [https://cef-builds.spotifycdn.com/](https://cef-builds.spotifycdn.com/).

### Step 2: Build the Project

#### macOS

On macOS, you need to create proper app bundles for CEF to function correctly:

```bash
# Build and bundle everything (helper app + framework)
cargo xtask bundle-all

# Or build components individually:
cargo xtask bundle-app        # Build the helper subprocess app
cargo xtask bundle-framework  # Build the GDExtension framework

# For release builds, add the --release flag:
cargo xtask bundle-all --release
```

This creates:
- `target/debug/Godot CEF.app/` — The CEF helper app with all required frameworks
- `target/debug/Godot CEF.framework/` — The GDExtension library bundle

#### Windows / Linux

```bash
# Build the GDExtension library
cargo build --lib

# Build the helper subprocess
cargo build --bin gdcef_helper

# For release builds:
cargo build --lib --release
cargo build --bin gdcef_helper --release
```

### Step 3: Copy to Your Godot Project

Copy the built artifacts to your Godot project's addon folder:

```
your-godot-project/
└── addons/
    └── godot_cef/
        └── bin/
            └── <platform>/
                ├── Godot CEF.framework/     # (macOS: GDExtension)
                ├── Godot CEF.app/           # (macOS: Helper app + CEF framework)
                ├── libgdcef.so              # (Linux: GDExtension)
                ├── gdcef.dll                # (Windows: GDExtension)
                └── gdcef_helper[.exe]       # (Windows/Linux: Helper)
```

## 🚀 Usage

Once installed, you can use the `CefTexture` node in your Godot scenes:

```gdscript
extends Control

func _ready():
    var cef_texture = CefTexture.new()
    cef_texture.url = "https://example.com"
    cef_texture.enable_accelerated_osr = true  # Enable GPU acceleration
    add_child(cef_texture)
```

### Node Properties

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `url` | `String` | `"https://google.com"` | The URL to load |
| `enable_accelerated_osr` | `bool` | `true` | Enable GPU-accelerated rendering |

### IME Methods

For input method editor (IME) support in text fields:

```gdscript
cef_texture.ime_commit_text("文字")        # Commit composed text
cef_texture.ime_set_composition("入力中")   # Set composition string
cef_texture.ime_cancel_composition()        # Cancel composition
cef_texture.ime_finish_composing_text(false) # Finish composing
```

## 🛣️ Roadmap

- [ ] Automatic Building Support
- [ ] CI/CD Configuration
- [ ] Custom Scheme Support
- [x] IPC Support
- [ ] Better IME Support
- [ ] Gamepad Support
- [ ] Access to Godot Filesystem

## 📄 License

MIT License — Copyright 2025-2026 Delton Ding

See [LICENSE](LICENSE) for details.

## 🙏 Acknowledgments

- [godot_wry](https://github.com/doceazedo/godot_wry)
- [gdcef](https://github.com/Lecrapouille/gdcef)
- [CEF (Chromium Embedded Framework)](https://bitbucket.org/chromiumembedded/cef)
- [godot-rust](https://github.com/godot-rust/gdext)
- [cef-rs](https://github.com/tauri-apps/cef-rs)
