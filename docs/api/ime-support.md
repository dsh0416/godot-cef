# IME Support

CefTexture provides automatic Input Method Editor (IME) support for text input in web content. When you click on an input field in the browser, the system IME is automatically activated, allowing you to input text in languages like Chinese, Japanese, Korean, etc.

## How it Works

- When an input field gains focus in CEF, Godot's native IME is automatically activated
- The IME candidate window is positioned near the text cursor in the browser
- Composition text is forwarded to CEF in real-time
- When the input field loses focus, IME is automatically deactivated

## Platform Support

IME behavior depends on the underlying operating system and Godot's own IME support on that platform:

| Platform | IME Support | Notes |
|----------|-------------|-------|
| **Windows** | ✅ Yes | Full IME support via Windows IME API |
| **macOS** | ✅ Yes | Full IME support via macOS Input Methods |
| **Linux** | ⚠️ Partial | Depends on desktop environment and input method framework |

## Configuration Requirements

- You must have a system IME / input source configured and enabled for the languages you want to type
- IME appearance and candidate window positioning may vary between platforms and window managers
- On platforms where Godot does not expose native IME support, IME behavior in CefTexture may be limited or unavailable

## Usage

IME support works automatically without additional configuration in your code. Simply ensure that:

1. Your system has the appropriate input methods installed
2. The web content you're loading uses standard HTML input elements
3. Users can interact with the CefTexture node normally

```gdscript
# No special setup needed for IME
extends Control

@onready var browser = $CefTexture

func _ready():
    browser.url = "https://example.com/form"  # Page with text inputs
    # IME will work automatically when users click on input fields
```

## Troubleshooting

### IME Not Appearing

If IME doesn't appear when clicking on text inputs:

1. **Check system IME settings**: Ensure your OS has input methods configured
2. **Verify platform support**: Some Linux desktop environments have limited IME support
3. **Test with simple inputs**: Try with basic `<input>` or `<textarea>` elements
4. **Check Godot version**: Ensure you're using Godot 4.5+ for full IME integration

### Candidate Window Positioning

The IME candidate window positioning is handled automatically, but may vary:

- **Windows**: Usually appears near the text cursor
- **macOS**: Follows system IME positioning rules
- **Linux**: Depends on the input method framework (IBus, Fcitx, etc.)

## Advanced Usage

For games that need custom IME handling, you can combine CEF IME support with Godot's native IME API:

```gdscript
extends Control

@onready var browser = $CefTexture
@onready var ime_manager = $IMEManager  # Custom IME handling node

func _ready():
    browser.url = "https://game-chat.example.com"

    # Monitor focus changes if needed
    browser.ipc_message.connect(_on_browser_message)

func _on_browser_message(message: String):
    var data = JSON.parse_string(message)
    if data.get("type") == "input_focus_changed":
        var has_focus = data.get("focused", false)
        # Handle custom IME logic if needed
        _update_ime_state(has_focus)
```

## Platform-Specific Notes

### Windows
- Full IME support through Windows IME API
- Supports all installed input methods
- Candidate windows position correctly relative to text cursor

### macOS
- Integrated with macOS Input Methods framework
- Supports system-wide IME configuration
- Follows macOS IME positioning conventions

### Linux
- Support varies by desktop environment and input method framework
- Best results with IBus or Fcitx input methods
- May require additional system configuration for full IME support

::: tip
For the best IME experience, test your web content on each target platform and ensure users have their preferred input methods properly configured in their system settings.
:::
