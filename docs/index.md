---
layout: home

hero:
  name: Godot CEF
  text: High-Performance Chromium Integration
  tagline: Full web browser capabilities for Godot Engine 4.5+
#   TODO: image:
#     src: /logo.svg
#     alt: Godot CEF
  actions:
    - theme: brand
      text: Get Started
      link: /api/
    - theme: alt
      text: View on GitHub
      link: https://github.com/dsh0416/godot-cef

features:
  - icon: ⚡
    title: GPU-Accelerated Rendering
    details: Leverage hardware acceleration for maximum performance with smooth, high-framerate web content rendering
  - icon: 🌐
    title: Full Web Standards Support
    details: Support for modern JavaScript, HTML5, and CSS3, compatible with the latest web technologies and frameworks
  - icon: 🔄
    title: Bidirectional IPC
    details: Seamless communication between Godot and JavaScript for easy integration of game logic with web UI
  - icon: 🖥️
    title: Cross-Platform Compatibility
    details: Native performance on Windows, macOS, and Linux with consistent behavior across all platforms
  - icon: ⌨️
    title: IME Support
    details: Complete Input Method Editor support for handling multi-language text input including CJK languages
  - icon: 🛠️
    title: Remote Debugging
    details: Debug with Chrome DevTools for streamlined development and testing workflows
---

## Quick Example

Integrate CEF browser into your Godot project with just a few lines of code:

```gdscript
extends Control

func _ready():
    var cef_texture = CefTexture.new()
    cef_texture.url = "https://example.com"
    cef_texture.enable_accelerated_osr = true
    add_child(cef_texture)
```

## Get Started

- [Installation Guide](https://github.com/dsh0416/godot-cef#installation) - Learn how to install and build Godot CEF
- [API Reference](./api/) - Complete documentation for CefTexture methods, properties, and signals
- [Usage Examples](https://github.com/dsh0416/godot-cef#usage) - Basic usage examples and best practices
- [Platform Support](https://github.com/dsh0416/godot-cef#platform-support-matrix) - View compatibility across platforms
