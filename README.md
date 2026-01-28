![Header](./assets/github-header-banner.png)

# Godot CEF

A high-performance Chromium Embedded Framework (CEF) integration for Godot Engine 4.5+, written in Rust. Render web content directly inside your Godot games and applications with full support for modern web standards, JavaScript, HTML5, and CSS3.

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

## Screenshots

| | |
|:---:|:---:|
| ![GitHub rendered in Godot](./assets/screenshot_1.png) | ![Web content as 3D texture](./assets/screenshot_2.png) |
| GitHub page rendered with full interactivity | Web content integrated into 3D scenes |
| ![WebGPU Samples](./assets/screenshot_3.png) | ![WebGL Aquarium](./assets/screenshot_4.png) |
| WebGPU demos running natively | WebGL Aquarium at ~120 FPS with 10,000 fish |

## Quick Start

### Installation

Download the latest pre-built binaries from the [Releases](https://github.com/dsh0416/godot-cef/releases) page, extract the addon to your Godot project's `addons/` folder, and you're ready to go!

### Basic Usage

```gdscript
extends Control

func _ready():
    var cef_texture = CefTexture.new()
    cef_texture.url = "https://example.com"
    cef_texture.enable_accelerated_osr = true  # Enable GPU acceleration
    add_child(cef_texture)
```

### Example with Signals

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

| Resource | Description |
|----------|-------------|
| [**API Reference**](https://dsh0416.github.io/godot-cef/api/) | Complete CefTexture API documentation |
| [**Properties**](https://dsh0416.github.io/godot-cef/api/properties.html) | Node properties and configuration |
| [**Methods**](https://dsh0416.github.io/godot-cef/api/methods.html) | Browser control and JavaScript execution |
| [**Signals**](https://dsh0416.github.io/godot-cef/api/signals.html) | Events and notifications |
| [**IME Support**](https://dsh0416.github.io/godot-cef/api/ime-support.html) | International text input |

## Platform Support

| Platform | DirectX 12 | Metal | Vulkan | Software Rendering |
|----------|------------|-------|--------|-------------------|
| **Windows** | ✅ (Note 1) | n.a. | ✅ (Note 2) | ✅ |
| **macOS** | n.a. | ✅ | ❌ [[#4]](https://github.com/dsh0416/godot-cef/issues/4) | ✅ |
| **Linux** | n.a. | n.a. | ✅ (Note 2) | ✅ |

<details>
<summary><strong>Platform Notes</strong></summary>

1. **Windows DirectX 12**: Requires at least Godot 4.6 beta 2. Godot 4.5.1 contains a bug where `RenderingDevice.get_driver_resource` on DirectX 12 textures always returns 0.

2. **Vulkan Backends**: See [#4](https://github.com/dsh0416/godot-cef/issues/4) for details. On Windows and Linux, we use hooking to inject extensions for GPU-accelerated rendering (x86_64 only). This is a workaround until [godotengine/godot-proposals#13969](https://github.com/godotengine/godot-proposals/issues/13969) is resolved.

3. **Software Rendering**: On platforms where accelerated rendering is not yet implemented, the extension automatically falls back to software rendering using CPU-based frame buffers.

</details>

## Building from Source

For detailed build instructions, see [CONTRIBUTING.md](CONTRIBUTING.md#development-setup).

### Quick Build Steps

1. **Install prerequisites**: Rust (nightly) and Godot 4.5+

2. **Install CEF binaries**:
   ```bash
   cargo install export-cef-dir
   export-cef-dir --version "144.0.11" --force $HOME/.local/share/cef
   export CEF_PATH="$HOME/.local/share/cef"
   ```

3. **Build**:
   ```bash
   cargo xtask bundle --release
   ```

4. **Copy to Godot project**: Copy built artifacts from `target/release/` to your project's `addons/godot_cef/bin/<platform>/` folder.

See the `addons/godot_cef/godot_cef.gdextension` file for the complete list of required files per platform.

## Comparison with Similar Projects

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

<details>
<summary><strong>When to Use Each</strong></summary>

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

</details>

### Motivation

This project was created during development of [Engram](https://store.steampowered.com/app/3928930/_Engram/). While our first demo version benefited greatly from an interactive UI written in Vue.js using godot_wry, we encountered limitations with the wry-based approach. Since other implementations have long struggled with GPU-accelerated OSR, we decided to create our own solution.

## Contributing

We welcome contributions! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines on:

- Setting up your development environment
- Code style and testing requirements
- Pull request process
- Reporting issues

## License

MIT License — Copyright 2025-2026 Delton Ding

See [LICENSE](LICENSE) for details.

## Acknowledgments

- [godot_wry](https://github.com/doceazedo/godot_wry)
- [gdcef](https://github.com/Lecrapouille/gdcef)
- [CEF (Chromium Embedded Framework)](https://bitbucket.org/chromiumembedded/cef)
- [godot-rust](https://github.com/godot-rust/gdext)
- [cef-rs](https://github.com/tauri-apps/cef-rs)

## Star History

<a href="https://www.star-history.com/#dsh0416/godot-cef&type=timeline&legend=top-left">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=dsh0416/godot-cef&type=timeline&theme=dark&legend=top-left" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=dsh0416/godot-cef&type=timeline&legend=top-left" />
   <img alt="Star History Chart" src="https://api.star-history.com/svg?repos=dsh0416/godot-cef&type=timeline&legend=top-left" />
 </picture>
</a>
