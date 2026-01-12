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
| **Windows** | ✅ (Note 1) | n.a. | ❌ [[#4]](https://github.com/dsh0416/godot-cef/issues/4) | ✅ |
| **macOS** | n.a. | ✅ | ❌ [[#4]](https://github.com/dsh0416/godot-cef/issues/4) | ✅ |
| **Linux** | n.a. | n.a. | ❌ [[#4]](https://github.com/dsh0416/godot-cef/issues/4) | ✅ |

### Note
1. For Windows DirectX 12 backend, it requires at least Godot 4.6 beta 2 to work. Since Godot 4.5.1 contains a bug when calling `RenderingDevice.get_driver_resource` on DirectX 12 textures ALWAYS returns 0.
2. On platforms where accelerated rendering is not yet implemented, the extension automatically falls back to software rendering using CPU-based frame buffers.

## 📦 Installation

### For Users

Download the latest pre-built binaries from the [Releases](https://github.com/nicholasccw/cef-godot/releases) page. Extract the addon to your Godot project's `addons/` folder and you're ready to go!

### For Developers

If you want to build from source or contribute to the project, follow the [build instructions](#-building-from-source) below.

## 🔄 Comparison with Similar Projects

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

## 🛠️ Building from Source

### Prerequisites

- **Rust** (1.92+) — Install via [rustup](https://rustup.rs/)
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
                # macOS (aarch64-apple-darwin)
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
| `url` | `String` | `"https://google.com"` | The URL to display. Setting this property navigates the browser to the new URL. Reading it returns the current URL from the browser. |
| `enable_accelerated_osr` | `bool` | `true` | Enable GPU-accelerated rendering |

The `url` property is reactive: when you set it from GDScript, the browser automatically navigates to the new URL:

```gdscript
# Navigate to a new page by setting the property
cef_texture.url = "https://example.com/game-ui"

# Read the current URL (reflects user navigation, redirects, etc.)
print("Currently at: ", cef_texture.url)
```

### Methods

#### Navigation

##### `go_back()`

Navigates back in the browser history.

```gdscript
cef_texture.go_back()
```

##### `go_forward()`

Navigates forward in the browser history.

```gdscript
cef_texture.go_forward()
```

##### `can_go_back() -> bool`

Returns `true` if the browser can navigate back.

```gdscript
if cef_texture.can_go_back():
    cef_texture.go_back()
```

##### `can_go_forward() -> bool`

Returns `true` if the browser can navigate forward.

```gdscript
if cef_texture.can_go_forward():
    cef_texture.go_forward()
```

##### `reload()`

Reloads the current page.

```gdscript
cef_texture.reload()
```

##### `reload_ignore_cache()`

Reloads the current page, ignoring any cached data.

```gdscript
cef_texture.reload_ignore_cache()
```

##### `stop_loading()`

Stops loading the current page.

```gdscript
cef_texture.stop_loading()
```

##### `is_loading() -> bool`

Returns `true` if the browser is currently loading a page.

```gdscript
if cef_texture.is_loading():
    print("Page is still loading...")
```

#### JavaScript Execution

##### `eval(code: String)`

Executes JavaScript code in the browser's main frame.

```gdscript
# Execute JavaScript
cef_texture.eval("document.body.style.backgroundColor = 'red'")

# Call a JavaScript function
cef_texture.eval("updateScore(100)")

# Interact with the DOM
cef_texture.eval("document.getElementById('player-name').innerText = 'Player1'")
```

#### IPC (Inter-Process Communication)

##### `send_ipc_message(message: String)`

Sends a message from Godot to JavaScript. The message will be delivered via `window.onIpcMessage(msg)` callback if it is registered.

```gdscript
# Send a simple string message
cef_texture.send_ipc_message("Hello from Godot!")

# Send structured data as JSON using a Dictionary
var payload := {"action": "update", "value": 42}
cef_texture.send_ipc_message(JSON.stringify(payload))
```

In your JavaScript (running in the CEF browser):

```javascript
// Register the callback to receive messages from Godot
window.onIpcMessage = function(msg) {
    console.log("Received from Godot:", msg);
    var data = JSON.parse(msg);
    // Handle the message...
};
```

#### Zoom Control

##### `set_zoom_level(level: float)`

Sets the zoom level for the browser. A value of `0.0` is the default (100%). Positive values zoom in, negative values zoom out.

```gdscript
cef_texture.set_zoom_level(1.0)   # Zoom in
cef_texture.set_zoom_level(-1.0)  # Zoom out
cef_texture.set_zoom_level(0.0)   # Reset to default
```

##### `get_zoom_level() -> float`

Returns the current zoom level.

```gdscript
var zoom = cef_texture.get_zoom_level()
print("Current zoom: ", zoom)
```

#### Audio Control

##### `set_audio_muted(muted: bool)`

Mutes or unmutes the browser audio.

```gdscript
cef_texture.set_audio_muted(true)   # Mute
cef_texture.set_audio_muted(false)  # Unmute
```

##### `is_audio_muted() -> bool`

Returns `true` if the browser audio is muted.

```gdscript
if cef_texture.is_audio_muted():
    print("Audio is muted")
```

### Signals

#### `ipc_message(message: String)`

Emitted when JavaScript sends a message to Godot via the `sendIpcMessage` function. Use this for bidirectional communication between your web UI and game logic.

```gdscript
func _ready():
    cef_texture.ipc_message.connect(_on_ipc_message)

func _on_ipc_message(message: String):
    print("Received from web: ", message)
    var data = JSON.parse_string(message)
    # Handle the message...
```

In your JavaScript (running in the CEF browser):

```javascript
// Send a message to Godot
window.sendIpcMessage("button_clicked");

// Send structured data as JSON
window.sendIpcMessage(JSON.stringify({ action: "purchase", item_id: 42 }));
```

#### `url_changed(url: String)`

Emitted when the browser navigates to a new URL. This fires for user-initiated navigation (clicking links), JavaScript navigation, redirects, and programmatic `load_url()` calls. Useful for injecting scripts or tracking navigation.

```gdscript
func _ready():
    cef_texture.url_changed.connect(_on_url_changed)

func _on_url_changed(url: String):
    print("Navigated to: ", url)
    # Inject data based on the current page
    if "game-ui" in url:
        cef_texture.eval("window.playerData = %s" % JSON.stringify(player_data))
```

#### `title_changed(title: String)`

Emitted when the page title changes. Useful for updating window titles or UI elements.

```gdscript
func _ready():
    cef_texture.title_changed.connect(_on_title_changed)

func _on_title_changed(title: String):
    print("Page title: ", title)
    $TitleLabel.text = title
```

#### `load_started(url: String)`

Emitted when the browser starts loading a page.

```gdscript
func _ready():
    cef_texture.load_started.connect(_on_load_started)

func _on_load_started(url: String):
    print("Loading: ", url)
    $LoadingSpinner.visible = true
```

#### `load_finished(url: String, http_status_code: int)`

Emitted when the browser finishes loading a page. The `http_status_code` contains the HTTP response status (e.g., 200 for success, 404 for not found).

```gdscript
func _ready():
    cef_texture.load_finished.connect(_on_load_finished)

func _on_load_finished(url: String, http_status_code: int):
    print("Loaded: ", url, " (status: ", http_status_code, ")")
    $LoadingSpinner.visible = false
    if http_status_code != 200:
        print("Warning: Page returned status ", http_status_code)
```

#### `load_error(url: String, error_code: int, error_text: String)`

Emitted when a page load error occurs (e.g., network error, invalid URL).

```gdscript
func _ready():
    cef_texture.load_error.connect(_on_load_error)

func _on_load_error(url: String, error_code: int, error_text: String):
    print("Failed to load: ", url)
    print("Error ", error_code, ": ", error_text)
    # Show error page or retry
```

### IME Support

CefTexture provides automatic Input Method Editor (IME) support for text input in web content. When you click on an input field in the browser, the system IME is automatically activated, allowing you to input text in languages like Chinese, Japanese, Korean, etc.

**How it works:**
- When an input field gains focus in CEF, Godot's native IME is automatically activated
- The IME candidate window is positioned near the text cursor in the browser
- Composition text is forwarded to CEF in real-time
- When the input field loses focus, IME is automatically deactivated

**Platform and configuration notes:**
- IME behavior depends on the underlying operating system and Godot's own IME support on that platform
- You must have a system IME / input source configured and enabled for the languages you want to type
- IME appearance and candidate window positioning may vary between platforms and window managers
- On platforms where Godot does not expose native IME support, IME behavior in CefTexture may be limited or unavailable

On platforms where Godot provides native IME integration, CefTexture works without additional configuration in this plugin. Refer to the platform support matrix above and the Godot documentation for details on per-platform IME support and any OS-level setup that may be required.
## 🛣️ Roadmap

- [x] Automatic Building Support
- [x] CI/CD Configuration
- [x] Custom Scheme Support (`res://` protocol)
- [x] IPC Support
- [x] Better IME Support
- [ ] Gamepad Support
- [x] Access to Godot Filesystem

## 📄 License

MIT License — Copyright 2025-2026 Delton Ding

See [LICENSE](LICENSE) for details.

## 🙏 Acknowledgments

- [godot_wry](https://github.com/doceazedo/godot_wry)
- [gdcef](https://github.com/Lecrapouille/gdcef)
- [CEF (Chromium Embedded Framework)](https://bitbucket.org/chromiumembedded/cef)
- [godot-rust](https://github.com/godot-rust/gdext)
- [cef-rs](https://github.com/tauri-apps/cef-rs)
