# Properties

The `CefTexture` node provides several properties for configuration and state management.

## Node Properties

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `url` | `String` | `"https://google.com"` | The URL to display. Setting this property navigates the browser to the new URL. Reading it returns the current URL from the browser. |
| `enable_accelerated_osr` | `bool` | `true` | Enable GPU-accelerated rendering |
| `background_color` | `Color` | `Color(0, 0, 0, 0)` | Background color for the browser. Set alpha to 0 for transparent background, or use a solid color to disable transparency. |
| `popup_policy` | `int` | `0` | Controls how popup windows are handled. `0` = BLOCK (suppress silently), `1` = REDIRECT (navigate current browser to popup URL), `2` = SIGNAL_ONLY (emit `popup_requested` signal). Can be changed at runtime. |

## Project Settings

Global settings that apply to **all** `CefTexture` instances are configured in **Project Settings > godot_cef**. These must be set before any `CefTexture` enters the scene tree.

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
| `godot_cef/network/enable_adblock` | `bool` | `false` | Enables request-level filtering using adblock rules for browsers created after this setting is configured (adblock configuration is determined at browser creation time). |
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

# Use software rendering (fallback for unsupported platforms)
cef_texture.enable_accelerated_osr = false
```

::: tip
GPU acceleration provides significantly better performance but may not be available on all platforms. The system automatically falls back to software rendering when accelerated rendering is unavailable.
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
