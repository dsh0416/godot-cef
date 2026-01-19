![Header](./assets/github-header-banner.png)

# Godot CEF

A high-performance Chromium Embedded Framework (CEF) integration for Godot Engine 4.5 and above, written in Rust. Render web content directly inside your Godot games and applications with full support for modern web standards, JavaScript, HTML5, and CSS3.

[![GitHub Actions Workflow Status](https://img.shields.io/github/actions/workflow/status/dsh0416/godot-cef/build.yml?label=Build)](https://github.com/dsh0416/godot-cef/actions/workflows/build.yml)
[![GitHub Actions Workflow Status](https://img.shields.io/github/actions/workflow/status/dsh0416/godot-cef/test.yml?label=Test)](https://github.com/dsh0416/godot-cef/actions/workflows/test.yml)
[![GitHub Release](https://img.shields.io/github/v/release/dsh0416/godot-cef)](https://github.com/dsh0416/godot-cef/releases)
[![GitHub Issues](https://img.shields.io/github/issues/dsh0416/godot-cef)](https://github.com/dsh0416/godot-cef/issues)
[![GitHub Pull Requests](https://img.shields.io/github/issues-pr/dsh0416/godot-cef)](https://github.com/dsh0416/godot-cef/pulls)

## Features

- **Web Rendering in Godot** — Display any web content as a texture using the `CefTexture` node (extends `TextureRect`)
- **Accelerated Off-Screen Rendering** — GPU-accelerated rendering using platform-native graphics APIs for maximum performance
- **Software Rendering Fallback** — Automatic fallback to CPU-based rendering when accelerated rendering is unavailable
- **Dynamic Scaling** — Automatic handling of DPI changes and window resizing
- **Multi-Process Architecture** — Proper CEF subprocess handling for stability and consistency
- **Remote Debugging** — Built-in Chrome DevTools support

## Platform Support Matrix

| Platform | DirectX 12 | Metal | Vulkan | Software Rendering |
|----------|---------------|-----------------|-------------------|--------|
| **Windows** | ✅ (Note 1) | n.a. | ✅ (Note 2) | ✅ |
| **macOS** | n.a. | ✅ | ❌ [[#4]](https://github.com/dsh0416/godot-cef/issues/4) | ✅ |
| **Linux** | n.a. | n.a. | ✅ (Note 2) | ✅ |

### Note
1. For Windows DirectX 12 backend, it requires at least Godot 4.6 beta 2 to work. Since Godot 4.5.1 contains a bug when calling `RenderingDevice.get_driver_resource` on DirectX 12 textures ALWAYS returns 0.
2. For Vulkan backends, see [[#4]](https://github.com/dsh0416/godot-cef/issues/4) for details. For Windows and Linux, we use hooking to inject extensions to enable GPU-accelerated rendering (x86_64 only). This is a dirty hack, until [godotengine/godot-proposals#13969](https://github.com/godotengine/godot-proposals/issues/13969) is solved.
3. On platforms where accelerated rendering is not yet implemented, the extension automatically falls back to software rendering using CPU-based frame buffers.

## Installation

### For Users

Download the latest pre-built binaries from the [Releases](https://github.com/dsh0416/godot-cef/releases) page. Extract the addon to your Godot project's `addons/` folder and you're ready to go!

### For Developers

If you want to build from source or contribute to the project, follow the [build instructions](#-building-from-source) below.

## Comparison with Similar Projects

There are several projects that bring web content into Godot. Here's how this project compares:

| Feature | **Godot CEF** (this project) | [godot_wry](https://github.com/doceazedo/godot_wry) | [gdcef](https://github.com/Lecrapouille/gdcef) |
|---------|------------------------------|-----------------------------------------------------|------------------------------------------------|
| **Browser Engine** | Chromium (CEF) | Native OS webview (WRY) | Chromium (CEF) |
| **Implementation** | Rust | Rust | C++ |
| **Rendering** | Texture (OSR) | Window overlay | Texture (OSR) |
| **GPU Acceleration** | ✅ Yes | ✅ Yes | ❌ Software only |
| **3D Scene Support** | ✅ Yes | ❌ No (always on top) | ✅ Yes |
| **HiDPI Aware** | ✅ Yes | ✅ Yes | ❌ No |
| **Consistent Cross-Platform** | ✅ Same engine everywhere | ❌ Different engines | ✅ Same engine everywhere |
| **JS ↔ GDScript IPC** | ✅ Yes | ✅ Yes | ✅ Yes |
| **Godot Filesystem Access** | ✅ Yes (`res://`) | ✅ Yes | ❌ No |
| **Project Export** | ✅ Yes | ✅ Yes | ❌ No |
| **Headless CI Support** | ✅ Yes | ❌ No | ✅ Yes |
| **Bundle Size** | Large (~100MB+) | Small (uses OS webview) | Large (~100MB+) |

### When to Use Each

**Choose Godot CEF (this project) if you need:**
- GPU-accelerated web rendering for high performance
- Smooth and high performance interactive UI
- Web content as a texture in 3D scenes (e.g., in-game screens, VR/AR interfaces)
- Consistent behavior across all platforms (same Chromium engine everywhere)
- Modern Rust codebase with godot-rust

**Choose godot_wry if you need:**
- Minimal bundle size (uses the OS's built-in webview)
- Simple overlay UI that doesn't need to be part of the 3D scene
- Lightweight integration without bundling a full browser

**Choose gdcef if you need:**
- C++ codebase for a more mature CEF integration with more docs
- Proven, mature implementation with longer history

### Motivation

The motivation for developing this project comes from our work-in-progress game, [Engram](https://store.steampowered.com/app/3928930/_Engram/). While our first demo version benefited greatly from an interactive UI written in Vue.js using godot_wry, we encountered the limitations of a wry-based browser solution. Since other implementations have long struggled with GPU-accelerated OSR, we decided to create our own solution.

## Building from Source

### Prerequisites

- **Rust** (nightly) — Install via [rustup](https://rustup.rs/)
- **Godot** (4.5+) — Download from [godotengine.org](https://godotengine.org/)
- **CEF Binaries** — Automatically downloaded during build

### Step 1: Install the CEF Export Tool

```bash
cargo install export-cef-dir
```

Then install the CEF frameworks

#### Linux
```bash
export-cef-dir --version "143.0.14" --force $HOME/.local/share/cef
export CEF_PATH="$HOME/.local/share/cef"
export LD_LIBRARY_PATH="$LD_LIBRARY_PATH:$CEF_PATH"
```

#### macOS
```bash
export-cef-dir --version "143.0.14" --force $HOME/.local/share/cef
export CEF_PATH="$HOME/.local/share/cef"
export LD_LIBRARY_PATH="$LD_LIBRARY_PATH:$CEF_PATH"

export-cef-dir --version "143.0.14" --target x86_64-apple-darwin --force $HOME/.local/share/cef_x86_64
export CEF_PATH_X64="$HOME/.local/share/cef_x86_64"
export-cef-dir --version "143.0.14" --target aarch64-apple-darwin --force $HOME/.local/share/cef_arm64
export CEF_PATH_ARM64="$HOME/.local/share/cef_arm64"
```

#### Windows
```powershell
export-cef-dir --version "143.0.14" --force $env:USERPROFILE/.local/share/cef
$env:CEF_PATH="$env:USERPROFILE/.local/share/cef"
$env:PATH="$env:PATH;$env:CEF_PATH"
```

This tool downloads and extracts the correct CEF binaries for your platform. For cross-platform building, download from [https://cef-builds.spotifycdn.com/](https://cef-builds.spotifycdn.com/).

### Step 2: Build the Project

The xtask build system works on all platforms and automatically bundles CEF assets:

```bash
# Build and bundle everything for your platform
cargo xtask bundle

# For release builds:
cargo xtask bundle --release
```

#### Platform-Specific Details

**macOS:**
- Creates `target/debug/Godot CEF.app/` — The CEF helper app with all required frameworks
- Creates `target/debug/Godot CEF.framework/` — The GDExtension library bundle
- Additional commands available:
  ```bash
  cargo xtask bundle-app        # Build only the helper subprocess app
  cargo xtask bundle-framework  # Build only the GDExtension framework
  ```

**Windows:**
- Builds `gdcef.dll` and `gdcef_helper.exe`
- Copies all required CEF DLLs and resources to `target/release/`

**Linux:**
- Builds `libgdcef.so` and `gdcef_helper`
- Copies all required CEF shared libraries and resources to `target/release/`

### Step 3: Copy to Your Godot Project

Copy the built artifacts from `target/release/` to your Godot project's addon folder:

```
your-godot-project/
└── addons/
    └── godot_cef/
        └── bin/
            └── <platform>/
                # macOS (universal-apple-darwin)
                ├── Godot CEF.framework/     # GDExtension library bundle
                └── Godot CEF.app/           # Helper app + CEF framework

                # Windows (x86_64-pc-windows-msvc)
                ├── gdcef.dll                # GDExtension library
                ├── gdcef_helper.exe         # Helper subprocess
                ├── libcef.dll               # CEF core library
                ├── locales/                 # Locale resources
                └── ...                      # Other CEF assets (see .gdextension)

                # Linux (x86_64-unknown-linux-gnu)
                ├── libgdcef.so              # GDExtension library
                ├── gdcef_helper             # Helper subprocess
                ├── libcef.so                # CEF core library
                ├── locales/                 # Locale resources
                └── ...                      # Other CEF assets (see .gdextension)
```

See `addons/godot_cef/godot_cef.gdextension` for the complete list of required files per platform.

## Usage

Once installed, you can use the `CefTexture` node in your Godot scenes:

```gdscript
extends Control

func _ready():
    var cef_texture = CefTexture.new()
    cef_texture.url = "https://example.com"
    cef_texture.enable_accelerated_osr = true  # Enable GPU acceleration
    add_child(cef_texture)
```

### Quick Example

```gdscript
extends Node2D

@onready var browser = $CefTexture

func _ready():
    # Set initial URL
    browser.url = "https://example.com"

    # Connect to signals
    browser.load_finished.connect(_on_page_loaded)
    browser.ipc_message.connect(_on_message_received)

func _on_page_loaded(url: String, status: int):
    print("Page loaded: ", url)

    # Execute JavaScript
    browser.eval("document.body.style.backgroundColor = '#f0f0f0'")

func _on_message_received(message: String):
    print("Received from web: ", message)
```

## Documentation

For comprehensive API documentation, examples, and guides, visit the [full documentation](https://dsh0416.github.io/godot-cef/).

- [**API Reference**](https://dsh0416.github.io/godot-cef/api/) - Complete CefTexture API documentation
- [**Properties**](https://dsh0416.github.io/godot-cef/api/properties.html) - Node properties and configuration
- [**Methods**](https://dsh0416.github.io/godot-cef/api/methods.html) - Browser control and JavaScript execution
- [**Signals**](https://dsh0416.github.io/godot-cef/api/signals.html) - Events and notifications
- [**IME Support**](https://dsh0416.github.io/godot-cef/api/ime-support.html) - International text input

## License

MIT License — Copyright 2025-2026 Delton Ding

See [LICENSE](LICENSE) for details.

## Acknowledgments

- [godot_wry](https://github.com/doceazedo/godot_wry)
- [gdcef](https://github.com/Lecrapouille/gdcef)
- [CEF (Chromium Embedded Framework)](https://bitbucket.org/chromiumembedded/cef)
- [godot-rust](https://github.com/godot-rust/gdext)
- [cef-rs](https://github.com/tauri-apps/cef-rs)
