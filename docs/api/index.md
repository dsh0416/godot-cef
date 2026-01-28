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
- **Drag-and-drop** between Godot and web content

## Global Configuration

Due to the architecture of CEF, certain parameters can only be configured **once** during Godot's boot-up process. These settings are configured via **Project Settings** and apply to all `CefTexture` instances.

### Project Settings

Navigate to **Project > Project Settings > godot_cef** to configure:

| Setting | Description |
|---------|-------------|
| `godot_cef/storage/data_path` | Path for cookies, cache, and localStorage (default: `user://cef-data`) |
| `godot_cef/security/allow_insecure_content` | Allow loading insecure (HTTP) content in HTTPS pages |
| `godot_cef/security/ignore_certificate_errors` | Ignore SSL/TLS certificate errors |
| `godot_cef/security/disable_web_security` | Disable web security (CORS, same-origin policy) |
| `godot_cef/audio/enable_audio_capture` | Route browser audio through Godot's audio system (default: `false`) |

These parameters are passed as command-line switches to the CEF subprocess during initialization and cannot be modified at runtime. If you need to change these settings, you must restart your Godot application.

**Note:** Remote debugging is also configured once at startup and is automatically enabled only when running in debug builds or from the Godot editor for security purposes.

::: warning
Security settings are dangerous and should only be enabled for specific use cases. Warnings will be logged at startup if any security settings are enabled.
:::

## API Sections

- [**Properties**](./properties.md) - Node properties and configuration
- [**Methods**](./methods.md) - Available methods for controlling the browser
- [**Signals**](./signals.md) - Events emitted by the CefTexture node
- [**Audio Capture**](./audio-capture.md) - Route browser audio through Godot's audio system
- [**IME Support**](./ime-support.md) - Input Method Editor integration
- [**Drag and Drop**](./drag-and-drop.md) - Bidirectional drag-and-drop support

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
