# API Reference

This section provides comprehensive documentation for the `CefTexture` node, which allows you to render web content as textures in your Godot scenes.

## Getting Started

Once the Godot CEF addon is installed, you can use the `CefTexture` node in your scenes:

::: info Packaging note
During export/package builds, Godot may convert some imported assets into other formats. If your frontend is built with Vite and needs specific source files to stay unchanged, consider using [`vite-plugin-godot-keep-import`](https://github.com/LemonNekoGH/vite-plugin-keep-import-for-godot) to keep imports for selected file types.
:::

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
- **Download handling** with full control over file downloads

## Global Configuration

Due to the architecture of CEF, certain parameters can only be configured **once** during Godot's boot-up process. These settings are configured via **Project Settings** and apply to all `CefTexture` instances.

### Project Settings

Navigate to **Project > Project Settings > godot_cef** to configure:

| Setting | Description |
|---------|-------------|
| `godot_cef/storage/data_path` | Path for cookies, cache, and localStorage (default: `user://cef-data`) |
| `godot_cef/storage/cache_size_mb` | Maximum disk cache size in MB (default: `0` = CEF default) |
| `godot_cef/security/allow_insecure_content` | Allow loading insecure (HTTP) content in HTTPS pages |
| `godot_cef/security/ignore_certificate_errors` | Ignore SSL/TLS certificate errors |
| `godot_cef/security/disable_web_security` | Disable web security (CORS, same-origin policy) |
| `godot_cef/audio/enable_audio_capture` | Route browser audio through Godot's audio system (default: `false`) |
| `godot_cef/debug/remote_devtools_port` | Port for Chrome DevTools remote debugging (default: `9229`) |
| `godot_cef/performance/max_frame_rate` | Maximum browser frame rate (default: `0` = follow Godot FPS) |
| `godot_cef/network/user_agent` | Custom user agent string (default: empty = CEF default) |
| `godot_cef/network/proxy_server` | Proxy server URL (default: empty = direct connection) |
| `godot_cef/network/proxy_bypass_list` | Hosts to bypass proxy (default: empty) |
| `godot_cef/advanced/custom_command_line_switches` | Custom CEF command-line switches (one per line) |

These parameters are passed as command-line switches to the CEF subprocess during initialization and cannot be modified at runtime. If you need to change these settings, you must restart your Godot application.

::: warning
Security settings are dangerous and should only be enabled for specific use cases. Warnings will be logged at startup if any security settings are enabled.
:::

## Remote DevTools

Remote DevTools allows you to debug web content running inside your Godot application using Chrome's Developer Tools. This is useful for inspecting the DOM, debugging JavaScript, monitoring network requests, and profiling performance.

### Availability

For security purposes, remote debugging is **only enabled** when:
- Godot is running in **debug mode** (`OS.is_debug_build()` returns `true`), OR
- Running from the **Godot editor** (`Engine.is_editor_hint()` returns `true`)

Remote debugging is automatically disabled in production/release builds.

### Accessing DevTools

When remote debugging is enabled, CEF listens on the configured port (default: **9229**). You can change this port via the `godot_cef/debug/remote_devtools_port` project setting.

1. Open Chrome and navigate to `chrome://inspect`
2. Click on **"Configure..."** next to "Discover network targets"
3. Add `localhost:<port>` to the target discovery list (e.g., `localhost:9229`)
4. Your CEF browser instances will appear under "Remote Target"
5. Click **"inspect"** to open DevTools for that page

### Use Cases

- **Debug JavaScript errors** in your web UI
- **Inspect and modify DOM** elements in real-time
- **Monitor network requests** to debug API calls
- **Profile performance** to identify bottlenecks
- **Test CSS changes** before applying them permanently

## API Sections

- [**Properties**](./properties.md) - Node properties and configuration
- [**Methods**](./methods.md) - Available methods for controlling the browser
- [**Signals**](./signals.md) - Events emitted by the CefTexture node
- [**Audio Capture**](./audio-capture.md) - Route browser audio through Godot's audio system
- [**IME Support**](./ime-support.md) - Input Method Editor integration
- [**Drag and Drop**](./drag-and-drop.md) - Bidirectional drag-and-drop support
- [**Downloads**](./downloads.md) - Handle file downloads from web pages
- [**Compatibility Matrix**](./compatibility-matrix.md) - Platform/backend acceleration and fallback behavior
- [**Production Security Baseline**](./security-baseline.md) - Recommended safe defaults for release builds

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

## Download Handling

```gdscript
func _ready():
    browser.download_requested.connect(_on_download_requested)
    browser.download_updated.connect(_on_download_updated)

func _on_download_requested(info: DownloadRequestInfo):
    print("Download: %s (%s)" % [info.suggested_file_name, info.mime_type])

func _on_download_updated(info: DownloadUpdateInfo):
    if info.is_complete:
        print("Download complete: ", info.full_path)
    elif info.is_in_progress:
        print("Progress: %d%%" % info.percent_complete)
```
