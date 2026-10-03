# Properties

The `CefTexture` node provides several properties for configuration and state management.
Internally, `CefTexture` now keeps an internal `CefTexture2D` helper for shared
settings/runtime-oriented operations, while still owning interaction-specific
features (input routing, IME proxy, popup overlay, and signal emission).

`CefTexture2D` is a render-first `Texture2D` resource variant and the shared
foundation for runtime/settings behavior. It can be assigned directly to
`Sprite2D.texture` and 3D material texture slots.

## Node Properties

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `url` | `String` | `"https://google.com"` | The URL to display. Setting this property navigates the browser to the new URL. Reading it returns the current URL from the browser. |
| `enable_accelerated_osr` | `bool` | `true` | Enable GPU-accelerated rendering |
| `background_color` | `Color` | `Color(0, 0, 0, 0)` | Background color for the browser. Set alpha to 0 for transparent background, or use a solid color to disable transparency. |
| `popup_policy` | `int` | `0` | Controls how popup windows are handled. `0` = BLOCK (suppress silently), `1` = REDIRECT (navigate current browser to popup URL), `2` = SIGNAL_ONLY (emit `popup_requested` signal). Can be changed at runtime. |
| `permission_policy` | `int` | `-1` | Inherit the project permission policy (`-1`), deny (`0`), allow known requests (`1`), or emit permission signals (`2`). Runtime changes cancel pending requests. See [Permissions](./permissions.md). |
| `preload_script` | `String` | `""` | JavaScript source executed once for the browser's main frame after the JS bridge is registered and before the document loads. Mutually exclusive with `preload_script_path`. |
| `preload_script_path` | `String` | `""` | Godot file path for JavaScript source to preload. Supports Godot paths such as `res://` and `user://`. Mutually exclusive with `preload_script`. |

## CefTexture2D Properties

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `url` | `String` | `"https://google.com"` | URL loaded by the resource-backed browser instance. |
| `enable_accelerated_osr` | `bool` | `true` | Enables GPU-accelerated OSR. Set to `false` to use software rendering. |
| `background_color` | `Color` | `Color(0, 0, 0, 0)` | Browser background color (supports transparency). |
| `popup_policy` | `int` | `0` | Popup behavior policy: BLOCK/REDIRECT/SIGNAL_ONLY. |
| `permission_policy` | `int` | `-1` | Same permission policy and signals as `CefTexture`; runtime changes cancel pending requests. See [Permissions](./permissions.md). |
| `preload_script` | `String` | `""` | JavaScript source executed once for the browser's main frame after the JS bridge is registered and before the document loads. Mutually exclusive with `preload_script_path`. |
| `preload_script_path` | `String` | `""` | Godot file path for JavaScript source to preload. Supports Godot paths such as `res://` and `user://`. Mutually exclusive with `preload_script`. |
| `texture_size` | `Vector2i` | `Vector2i(1024, 1024)` | Logical browser texture size in pixels. |

`CefTexture2D` remains focused on rendering and does not include built-in
3D surface input mapping/raycast routing. It exposes `permission_requested` and
`permission_request_finished`, with the same permission methods as `CefTexture`.
It does not expose the node's other browser event signals, such as
`load_finished`, `title_changed`, `console_message`, or `popup_requested`.
For loading notifications, use a `CefTexture` node and connect to its signals.
Its `texture` property can be used by a `Sprite2D` or material when needed.

`CefTexture2D` does provide optional low-level `forward_*` input helper methods.
These helpers do not perform node-space coordinate mapping; callers must provide
already-mapped positions and explicit scale factors.

```gdscript
var browser_tex := CefTexture2D.new()
browser_tex.url = "https://example.com"
browser_tex.texture_size = Vector2i(1024, 1024)
$Sprite2D.texture = browser_tex
```

```gdscript
var browser_tex := CefTexture2D.new()
browser_tex.url = "https://example.com"
var mat := StandardMaterial3D.new()
mat.albedo_texture = browser_tex
$MeshInstance3D.set_surface_override_material(0, mat)
```

## Preload Scripts

Use `preload_script` or `preload_script_path` to install trusted app-provided
JavaScript before the page document loads. Preload runs after Godot CEF
registers its built-in JavaScript bridge, so APIs such as
`window.sendIpcMessage(...)` are available inside the preload script.

Set preload properties before the browser instance is created. For `CefTexture`,
this means before the node enters the scene tree or before its browser is
otherwise initialized. For `CefTexture2D`, set them before the resource-backed
browser is initialized by the scene using it. For a `CefTexture` already present
in a scene, set these properties in the Inspector.

```gdscript
func _ready() -> void:
    var browser := CefTexture.new()
    browser.preload_script = """
        window.appConfig = { locale: "en-US" };
        window.sendIpcMessage("preload-ready");
    """
    browser.url = "res://ui/index.html"
    add_child(browser)
```

For larger scripts, store the JavaScript in a file and set
`preload_script_path`:

```gdscript
browser.preload_script_path = "res://browser/preload.js"
browser.url = "https://example.com"
```

`preload_script` and `preload_script_path` are mutually exclusive. If both are
non-empty, browser creation fails with an error. If `preload_script_path` cannot
be opened or is not valid UTF-8, browser creation also fails.

Preload only runs in the main frame. It does not run in iframes.

::: warning Trusted code only
Preload executes in the page's main JavaScript world. It is intended for trusted
application setup, such as installing app globals or wiring IPC. It is not a
security isolation boundary, and it should not contain secrets that the loaded
page must not read.
:::

## Project Settings

Global settings that apply to **all** `CefTexture` and `CefTexture2D` instances are configured in **Project Settings > godot_cef**. These must be set before any `CefTexture` enters the scene tree.

### Storage Settings

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `godot_cef/storage/data_path` | `String` | `"user://cef-data"` | Path for cookies, cache, and localStorage. Supports `user://` and `res://` protocols. |

### Security Settings

::: danger Security Warning
These settings are dangerous and should only be enabled for specific use cases (e.g., loading local development content). Enabling these in production can expose users to security vulnerabilities.
:::

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `godot_cef/security/allow_insecure_content` | `bool` | `false` | Allow loading HTTP content in HTTPS pages |
| `godot_cef/security/ignore_certificate_errors` | `bool` | `false` | Skip SSL/TLS certificate validation |
| `godot_cef/security/disable_web_security` | `bool` | `false` | Disable CORS and same-origin policy |
| `godot_cef/security/default_permission_policy` | `int` | `0` | Default permission behavior. `0` = DENY_ALL, `1` = ALLOW_ALL, `2` = SIGNAL (emit `permission_requested`) |
| `godot_cef/security/permission_request_timeout_seconds` | `int` | `60` | Deadline for unanswered permission requests, in seconds (minimum `1`). Configure before browser creation. |

The permission policy can be overridden per browser with `permission_policy`.
See [Permissions](./permissions.md) for request grouping, cleanup, and examples.

### Debug Settings

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `godot_cef/debug/remote_devtools_port` | `int` | `9229` | Port for Chrome DevTools remote debugging. Only active in debug builds or when running from the editor. |

### Performance Settings

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `godot_cef/performance/max_frame_rate` | `int` | `0` | Maximum frame rate for browser rendering. Set to `0` to follow Godot engine's FPS setting. Valid range: 1-240+. |

### Cache Settings

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `godot_cef/storage/cache_size_mb` | `int` | `0` | Maximum disk cache size in megabytes. Set to `0` for CEF default. |

### Network Settings

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `godot_cef/network/user_agent` | `String` | `""` | Custom user agent string. Leave empty to use CEF's default user agent. |
| `godot_cef/network/proxy_server` | `String` | `""` | Proxy server URL (e.g., `socks5://127.0.0.1:1080` or `http://proxy:8080`). Leave empty for direct connection. |
| `godot_cef/network/proxy_bypass_list` | `String` | `""` | Comma-separated list of hosts to bypass proxy (e.g., `localhost,127.0.0.1,*.local`). |
| `godot_cef/network/enable_adblock` | `bool` | `false` | Enables request-level filtering using adblock rules. Configure this before the first browser is created; all browser instances share the resulting request context. |
| `godot_cef/network/adblock_rules_path` | `String` | `""` | Path to an EasyList/ABP-compatible rules file. Supports `user://` and `res://`. Ignored when adblock is disabled. |

### Advanced Settings

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `godot_cef/advanced/custom_command_line_switches` | `String` | `""` | Custom CEF command-line switches (one per line). Prefix with `#` to comment out. Format: `switch-name` or `switch-name=value`. |

::: danger Security Warning
The custom command-line switches setting allows you to pass additional CEF/Chromium flags, including ones that can disable important security features (for example, `disable-web-security` or `allow-running-insecure-content`). Use this setting **only** if you fully understand the implications of each switch, and never enable insecure flags for untrusted content or in production builds.

Each line should contain one switch. Lines starting with `#` are ignored. Examples:
- `disable-gpu-compositing`
- `enable-features=WebRTC`
- `js-flags=--max-old-space-size=4096`
:::

### Example Configuration

In your `project.godot` file:

```ini
[godot_cef]
storage/data_path="user://my-app-browser-data"
storage/cache_size_mb=512
security/allow_insecure_content=false
performance/max_frame_rate=60
network/user_agent="MyApp/1.0 (Godot Engine)"
network/proxy_server="socks5://127.0.0.1:1080"
network/proxy_bypass_list="localhost,127.0.0.1"
network/enable_adblock=true
network/adblock_rules_path="user://filters/easylist.txt"
advanced/custom_command_line_switches="disable-gpu-compositing\nenable-features=WebRTC"
```

Or configure via GDScript before any CefTexture is created:

```gdscript
# In an autoload or early-loading script
func _init():
    ProjectSettings.set_setting("godot_cef/storage/data_path", "user://custom-cef-data")
```

## URL Property

The `url` property is reactive: when you set it from GDScript, the browser automatically navigates to the new URL:

```gdscript
# Navigate to a new page by setting the property
cef_texture.url = "https://example.com/game-ui"

# Read the current URL (reflects user navigation, redirects, etc.)
print("Currently at: ", cef_texture.url)
```

## Accelerated OSR

The `enable_accelerated_osr` property controls whether GPU acceleration is used for rendering:

```gdscript
# Enable GPU-accelerated rendering (recommended for performance)
cef_texture.enable_accelerated_osr = true

# Use software rendering
cef_texture.enable_accelerated_osr = false
```

::: tip
See the [supported backend matrix](./compatibility-matrix) and [Accelerated frame handoff](./accelerated-handoff) for native GPU synchronization and resource lifetime details.
:::

## Background Color

The `background_color` property controls the browser's background color. Set alpha to `0` for transparency.

```gdscript
# Transparent background (default)
cef_texture.background_color = Color(0, 0, 0, 0)

# Solid background
cef_texture.background_color = Color(0.2, 0.3, 0.4, 1)
```

## Popup Policy

The `popup_policy` property controls how popup windows (`window.open()`, `target="_blank"` links) are handled. It can be changed at runtime and takes effect immediately for subsequent popup requests.

| Value | Name | Behavior |
|-------|------|----------|
| `0` | BLOCK | Suppress all popups silently (default, backward-compatible) |
| `1` | REDIRECT | Navigate the current browser to the popup URL instead of opening a new window |
| `2` | SIGNAL_ONLY | Emit the `popup_requested` signal and let GDScript decide |

```gdscript
# Block all popups (default)
cef_texture.popup_policy = 0

# Automatically follow popup links in the same browser
cef_texture.popup_policy = 1

# Handle popups in GDScript
cef_texture.popup_policy = 2
cef_texture.popup_requested.connect(func(url, disposition, user_gesture):
    if user_gesture:
        cef_texture.url = url  # Follow user-initiated popups
)
```

::: tip
The REDIRECT policy is the simplest option for single-browser setups — it turns `target="_blank"` links into regular navigation. Use SIGNAL_ONLY when you need fine-grained control (e.g., blocking ads while allowing user-initiated popups).
:::
