# Methods

The `CefTexture` node provides comprehensive methods for controlling browser behavior and interacting with web content.

## Navigation

### `go_back()`

Navigates back in the browser history.

```gdscript
cef_texture.go_back()
```

### `go_forward()`

Navigates forward in the browser history.

```gdscript
cef_texture.go_forward()
```

### `can_go_back() -> bool`

Returns `true` if the browser can navigate back.

```gdscript
if cef_texture.can_go_back():
    cef_texture.go_back()
```

### `can_go_forward() -> bool`

Returns `true` if the browser can navigate forward.

```gdscript
if cef_texture.can_go_forward():
    cef_texture.go_forward()
```

### `reload()`

Reloads the current page.

```gdscript
cef_texture.reload()
```

### `reload_ignore_cache()`

Reloads the current page, ignoring any cached data.

```gdscript
cef_texture.reload_ignore_cache()
```

### `stop_loading()`

Stops loading the current page.

```gdscript
cef_texture.stop_loading()
```

### `is_loading() -> bool`

Returns `true` if the browser is currently loading a page.

```gdscript
if cef_texture.is_loading():
    print("Page is still loading...")
```

## JavaScript Execution

### `eval(code: String)`

Executes JavaScript code in the browser's main frame.

```gdscript
# Execute JavaScript
cef_texture.eval("document.body.style.backgroundColor = 'red'")

# Call a JavaScript function
cef_texture.eval("updateScore(100)")

# Interact with the DOM
cef_texture.eval("document.getElementById('player-name').innerText = 'Player1'")
```

## IPC (Inter-Process Communication)

### `send_ipc_message(message: String)`

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

## Zoom Control

### `set_zoom_level(level: float)`

Sets the zoom level for the browser. A value of `0.0` is the default (100%). Positive values zoom in, negative values zoom out.

```gdscript
cef_texture.set_zoom_level(1.0)   # Zoom in
cef_texture.set_zoom_level(-1.0)  # Zoom out
cef_texture.set_zoom_level(0.0)   # Reset to default
```

### `get_zoom_level() -> float`

Returns the current zoom level.

```gdscript
var zoom = cef_texture.get_zoom_level()
print("Current zoom: ", zoom)
```

## Audio Control

### `set_audio_muted(muted: bool)`

Mutes or unmutes the browser audio.

```gdscript
cef_texture.set_audio_muted(true)   # Mute
cef_texture.set_audio_muted(false)  # Unmute
```

### `is_audio_muted() -> bool`

Returns `true` if the browser audio is muted.

```gdscript
if cef_texture.is_audio_muted():
    print("Audio is muted")
```
