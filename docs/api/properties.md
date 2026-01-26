# Properties

The `CefTexture` node provides several properties for configuration and state management.

## Node Properties

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `url` | `String` | `"https://google.com"` | The URL to display. Setting this property navigates the browser to the new URL. Reading it returns the current URL from the browser. |
| `enable_accelerated_osr` | `bool` | `true` | Enable GPU-accelerated rendering |

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

### Example Configuration

In your `project.godot` file:

```ini
[godot_cef]
storage/data_path="user://my-app-browser-data"
security/allow_insecure_content=false
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
