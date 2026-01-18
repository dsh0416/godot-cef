# API Reference

This section provides comprehensive documentation for the `CefTexture` node, which allows you to render web content as textures in your Godot scenes.

## Getting Started

Once the Godot CEF addon is installed, you can use the `CefTexture` node in your scenes:

```gdscript
extends Control

func _ready():
    var cef_texture = CefTexture.new()
    cef_texture.url = "https://example.com"
    cef_texture.enable_accelerated_osr = true  # Enable GPU acceleration
    add_child(cef_texture)
```

## Overview

The `CefTexture` node extends `TextureRect` and provides a Chromium-based web browser rendered as a texture. It supports:

- **GPU-accelerated rendering** for high performance
- **Interactive web content** with full JavaScript support
- **Bidirectional communication** between Godot and JavaScript
- **Input handling** including mouse, keyboard, and IME support
- **Navigation controls** and browser state management

## ⚠️ Limitations

### One-Time Initialization Parameters

Due to the architecture of CEF, certain parameters can only be configured **once** during Godot's boot-up process. Once CEF is initialized, these settings cannot be changed without restarting the application.

The following security configuration options in `cef_app/src/lib.rs` are affected:

| Parameter | Description |
|-----------|-------------|
| `allow_insecure_content` | Allow loading insecure (HTTP) content in HTTPS pages |
| `ignore_certificate_errors` | Ignore SSL/TLS certificate errors |
| `disable_web_security` | Disable web security (CORS, same-origin policy) |

These parameters are passed as command-line switches to the CEF subprocess during initialization and cannot be modified at runtime. If you need to change these settings, you must restart your Godot application.

**Note:** Remote debugging is also configured once at startup and is automatically enabled only when running in debug builds or from the Godot editor for security purposes.

## API Sections

- [**Properties**](./properties.md) - Node properties and configuration
- [**Methods**](./methods.md) - Available methods for controlling the browser
- [**Signals**](./signals.md) - Events emitted by the CefTexture node
- [**IME Support**](./ime-support.md) - Input Method Editor integration

## Basic Usage Example

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

## Navigation

```gdscript
# Navigate to URLs
browser.url = "https://godotengine.org"

# Browser controls
if browser.can_go_back():
    browser.go_back()

if browser.can_go_forward():
    browser.go_forward()

browser.reload()
browser.reload_ignore_cache()
```
