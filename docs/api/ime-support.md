# IME Support

CefTexture provides automatic Input Method Editor (IME) support for text input in web content. When you click on an input field in the browser, the system IME is automatically activated, allowing you to input text in languages like Chinese, Japanese, Korean, etc.

## How it Works

### Automatic Activation
- When an editable element (input, textarea, or contentEditable) gains focus in CEF, Godot's native IME is automatically activated
- A hidden LineEdit proxy captures IME input and forwards composition text to CEF
- When the editable element loses focus, IME is automatically deactivated

### Caret Position Tracking
The IME candidate window is positioned near the text cursor through a dual-mechanism approach:

1. **JavaScript-based tracking**: A helper script injected into each page reports caret position on:
   - Initial focus on an editable element
   - Selection changes (clicking to reposition cursor)
   - Text input (including delete, paste, etc.)
   - Arrow key navigation

2. **CEF composition callback**: During active IME composition, CEF's `on_ime_composition_range_changed` callback provides precise caret bounds

Both mechanisms write to the same queue, ensuring the IME window stays correctly positioned throughout the editing session.

### Focus Handling
When clicking inside an already-focused editable element to reposition the cursor:
- The system detects focus transitioning to the parent CefTexture
- Focus is automatically re-grabbed on the IME proxy to maintain input capability
- This prevents IME from being incorrectly deactivated during cursor repositioning

## Configuration Requirements

- You must have a system IME / input source configured and enabled for the languages you want to type
- IME appearance and candidate window positioning may vary between platforms and window managers
- On platforms where Godot does not expose native IME support, IME behavior in CefTexture may be limited or unavailable

## Usage

IME support works automatically without additional configuration in your code. Simply ensure that:

1. Your system has the appropriate input methods installed
2. The web content you're loading uses standard HTML input elements (`<input>`, `<textarea>`, or `contentEditable`)
3. Users can interact with the CefTexture node normally

```gdscript
# No special setup needed for IME
extends Control

@onready var browser = $CefTexture

func _ready():
    browser.url = "https://example.com/form"  # Page with text inputs
    # IME will work automatically when users click on input fields
```

## Supported Element Types

| Element Type | Support |
|--------------|---------|
| `<input type="text">` | ✅ Full support |
| `<textarea>` | ✅ Full support |
| `contentEditable` elements | ✅ Full support |
| `<input type="password">` | ✅ Full support |
| Other input types | ⚠️ Varies by type |

## Troubleshooting

### IME window appears in wrong position
- Ensure you're using standard HTML input elements
- Custom-styled inputs with unusual CSS may affect caret position calculation
- Try clicking directly in the input field to trigger position recalculation

### IME doesn't activate
- Verify your system IME is properly configured
- Check that the element is a standard editable element
- Ensure CefTexture has focus in the Godot scene

### IME deactivates unexpectedly
- This may occur when clicking outside the editable element
- Clicking on non-editable areas of the page will deactivate IME as expected
