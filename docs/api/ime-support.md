# IME Support

CefTexture provides automatic Input Method Editor (IME) support for text input in web content. When you click on an input field in the browser, the system IME is automatically activated, allowing you to input text in languages like Chinese, Japanese, Korean, etc.

## How it Works

- When an input field gains focus in CEF, Godot's native IME is automatically activated
- CEF returns the proper position of the text cursor
- The IME candidate window is positioned near the text cursor in the browser
- Composition text is forwarded to CEF in real-time
- When the input field loses focus, IME is automatically deactivated

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
