# Display Events and Visibility

`CefTexture` and `CefTexture2D` expose the same five display signals:

| Signal | Value |
|--------|-------|
| `tooltip_changed(text: String)` | Tooltip text; an empty string clears the tooltip. |
| `fullscreen_changed(fullscreen: bool)` | Whether web content is in fullscreen mode. |
| `loading_progress_changed(progress: float)` | CEF's estimated load progress in the range `0.0`–`1.0`. |
| `status_message_changed(message: String)` | Browser status text, including the target URL of a hovered link; an empty string clears it. |
| `favicon_urls_changed(urls: PackedStringArray)` | Candidate favicon URLs; an empty array clears them. |

Connect these signals to your Godot UI. The integration does not create a native
tooltip or switch the Godot window to fullscreen automatically. Favicon events
provide URLs, not downloaded images. Progress is an estimate, rather than a count
of bytes or resources; use `CefTexture`'s load completion/error signals when you
need those outcomes.

## Godot UI Example

This example uses a `CefTexture` child named `Browser`, a `ProgressBar`, and two
labels named `StatusLabel` and `FaviconURLLabel`. The fullscreen handler shows one
possible application policy and restores the previous window mode on exit.

```gdscript
extends Control

@onready var browser: CefTexture = $Browser
var favicon_urls := PackedStringArray()
var _browser_fullscreen := false
var _previous_window_mode := Window.MODE_WINDOWED

func _ready() -> void:
    browser.tooltip_changed.connect(_on_tooltip_changed)
    browser.fullscreen_changed.connect(_on_fullscreen_changed)
    browser.loading_progress_changed.connect(_on_loading_progress_changed)
    browser.status_message_changed.connect(_on_status_message_changed)
    browser.favicon_urls_changed.connect(_on_favicon_urls_changed)
    $ProgressBar.max_value = 100.0

func _on_tooltip_changed(text: String) -> void:
    browser.tooltip_text = text

func _on_loading_progress_changed(progress: float) -> void:
    $ProgressBar.value = progress * 100.0
    $ProgressBar.visible = progress < 1.0

func _on_status_message_changed(message: String) -> void:
    $StatusLabel.text = message

func _on_favicon_urls_changed(urls: PackedStringArray) -> void:
    favicon_urls = urls
    $FaviconURLLabel.text = "" if urls.is_empty() else urls[0]
    # The application can choose a URL, fetch it, and decode a supported image.

func _on_fullscreen_changed(fullscreen: bool) -> void:
    if fullscreen == _browser_fullscreen:
        return
    _browser_fullscreen = fullscreen
    if fullscreen:
        _previous_window_mode = get_window().mode
        get_window().mode = Window.MODE_FULLSCREEN
    else:
        get_window().mode = _previous_window_mode
```

The signals also work with a `CefTexture2D` reference. In that case, apply the
tooltip text to the Godot `Control` displaying the texture. Unrelated node events,
such as `load_finished`, `title_changed`, and `console_message`, are still only
available on `CefTexture`.

## Hide and Resume Rendering

Both browser types have a `browser_hidden` property, defaulting to `false`.
Setting it notifies CEF that the off-screen browser is hidden and stops external
begin-frame requests. The latest texture remains available. Clearing it resumes
rendering without recreating or reloading the browser.

```gdscript
browser.browser_hidden = true
print(browser.get_browser_hidden())  # Manual setting: true.
print(browser.is_browser_hidden())   # Effective rendering state: true.
browser.set_browser_hidden(false)    # Resume, unless automatic hiding applies.
```

`CefTexture` additionally has `pause_rendering_when_hidden`, defaulting to `false`.
When enabled, rendering is hidden if `is_visible_in_tree()` is false, including
when an ancestor is hidden. The effective state is the manual setting OR the
automatic hidden state:

```gdscript
browser.pause_rendering_when_hidden = true
$BrowserPanel.hide()  # A hidden ancestor also hides browser rendering.
$BrowserPanel.show()  # Resume if browser_hidden is false.
```

`get_browser_hidden()` returns only the manual setting; `is_browser_hidden()`
includes this automatic policy. Manually setting `browser_hidden = true` keeps
rendering hidden even when the node is visible.

`CefTexture2D` has no automatic node visibility policy because a resource may be
shared by multiple sprites or materials. Its consumers explicitly set
`browser_hidden` when none of their views need rendering.

Hidden rendering keeps CEF task servicing, JavaScript, network activity, audio,
display signals, and browser lifecycle work alive. It does not suspend the
application or revoke permissions. Scene processing settings and main-loop
throttling still apply; use audio or permission controls separately when needed.
