# Godot CEF Documentation

Welcome to the official documentation for Godot CEF, a high-performance Chromium Embedded Framework integration for Godot Engine 4.5+.

## 🚀 Getting Started

Godot CEF allows you to render web content directly in your Godot games and applications using the `CefTexture` node.

### Quick Start

```gdscript
extends Control

func _ready():
    var cef_texture = CefTexture.new()
    cef_texture.url = "https://example.com"
    cef_texture.enable_accelerated_osr = true
    add_child(cef_texture)
```

## 📚 Documentation

- [**Installation Guide**](https://github.com/dsh0416/godot-cef#installation) - How to install and build Godot CEF
- [**API Reference**](./api/) - Complete reference for CefTexture methods, properties, and signals
- [**Platform Support**](https://github.com/dsh0416/godot-cef#platform-support-matrix) - Compatibility across different platforms
- [**Examples**](https://github.com/dsh0416/godot-cef#usage) - Basic usage examples

## 🎯 Key Features

- **GPU-accelerated rendering** for maximum performance
- **Full web standards support** including modern JavaScript, HTML5, and CSS3
- **Bidirectional IPC** between Godot and JavaScript
- **Cross-platform compatibility** with native performance on each platform
- **IME support** for international text input
- **Remote debugging** with Chrome DevTools

## 🏗️ Architecture

Godot CEF uses Chromium Embedded Framework (CEF) to provide full web browser capabilities within Godot. The system consists of:

- **Main Process**: Your Godot application
- **Helper Process**: CEF subprocess for browser rendering
- **Texture Rendering**: GPU-accelerated off-screen rendering to Godot textures

## 📖 API Reference

Dive deep into the CefTexture API:

- [Properties](./api/properties.md) - Configuration options
- [Methods](./api/methods.md) - Browser control and JavaScript execution
- [Signals](./api/signals.md) - Events and notifications
- [IME Support](./api/ime-support.md) - International text input

## 🤝 Contributing

Found a bug or want to contribute? Check out our [GitHub repository](https://github.com/dsh0416/godot-cef) and the [contributing guidelines](https://github.com/dsh0416/godot-cef#building-from-source).

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](https://github.com/dsh0416/godot-cef/blob/main/LICENSE) file for details.
