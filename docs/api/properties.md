# Properties

The `CefTexture` node provides several properties for configuration and state management.

## Node Properties

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `url` | `String` | `"https://google.com"` | The URL to display. Setting this property navigates the browser to the new URL. Reading it returns the current URL from the browser. |
| `enable_accelerated_osr` | `bool` | `true` | Enable GPU-accelerated rendering |

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
