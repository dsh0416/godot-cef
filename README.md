# Godot CEF

A high-performance Chromium Embedded Framework (CEF) integration for Godot Engine 4.4 and above, written in Rust. Render web content directly inside your Godot games and applications with full support for modern web standards, JavaScript, HTML5, and CSS3.

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
| **Windows** | ✅ | n.a. | 🚧 WIP | ✅ |
| **macOS** | n.a. | ✅ | ✅ (Note 1) | ✅ |
| **Linux** | n.a. | n.a. | 🚧 WIP | ✅ |

### Note
1. for macOS Vulkan backend, CEF would keep using `--use-angle=metal` to launch the process, since `--use-angle=vulkan` on macOS would cause GPU acceleration disabled in CEF, which causes performance and compatibility issues.
2. On platforms where accelerated rendering is not yet implemented, the extension automatically falls back to software rendering using CPU-based frame buffers.

## 🛠️ Prerequisites

- **Rust** (2024 Edition) — Install via [rustup](https://rustup.rs/)
- **Godot 4.4** — Download from [godotengine.org](https://godotengine.org/)
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
# Build and bundle the helper subprocess app
cargo run --bin bundle_app

# Build and bundle the GDExtension framework
cargo run --bin bundle_framework
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
- [ ] IPC Support
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
