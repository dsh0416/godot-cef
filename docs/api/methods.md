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

### `send_ipc_binary_message(data: PackedByteArray)`

Sends binary data from Godot to JavaScript. The data will be delivered as an `ArrayBuffer` via `window.onIpcBinaryMessage(arrayBuffer)` callback if it is registered.

Uses native CEF process messaging with zero encoding overhead for efficient binary data transfer (images, audio, protocol buffers, etc.).

```gdscript
# Send raw binary data
var data := PackedByteArray([0x01, 0x02, 0x03, 0x04])
cef_texture.send_ipc_binary_message(data)

# Send an image as binary
var image := Image.load_from_file("res://icon.png")
var png_data := image.save_png_to_buffer()
cef_texture.send_ipc_binary_message(png_data)

# Send a file's contents
var file := FileAccess.open("res://data.bin", FileAccess.READ)
var file_data := file.get_buffer(file.get_length())
cef_texture.send_ipc_binary_message(file_data)
```

In your JavaScript (running in the CEF browser):

```javascript
// Register the callback to receive binary data from Godot
window.onIpcBinaryMessage = function(arrayBuffer) {
    console.log("Received binary data:", arrayBuffer.byteLength, "bytes");
    
    // Example: Process as an image
    const blob = new Blob([arrayBuffer], { type: 'image/png' });
    const url = URL.createObjectURL(blob);
    document.getElementById('image').src = url;
    
    // Example: Process as typed array
    const view = new Uint8Array(arrayBuffer);
    console.log("First byte:", view[0]);
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

## Audio Capture

These methods enable routing browser audio through Godot's audio system. For comprehensive documentation, see the [Audio Capture](./audio-capture.md) page.

::: tip
Audio capture must be enabled in Project Settings (`godot_cef/audio/enable_audio_capture`) before browsers are created.
:::

### `is_audio_capture_enabled() -> bool`

Returns `true` if audio capture mode is enabled in project settings.

```gdscript
if cef_texture.is_audio_capture_enabled():
    print("Audio capture is enabled")
```

### `create_audio_stream() -> AudioStreamGenerator`

Creates and returns an `AudioStreamGenerator` configured with the correct sample rate.

```gdscript
var audio_stream = cef_texture.create_audio_stream()
audio_player.stream = audio_stream
audio_player.play()
```

### `push_audio_to_playback(playback: AudioStreamGeneratorPlayback) -> int`

Pushes buffered audio data from CEF to the given playback. Returns the number of frames pushed. Call this every frame in `_process()`.

```gdscript
func _process(_delta):
    var playback = audio_player.get_stream_playback()
    if playback:
        cef_texture.push_audio_to_playback(playback)
```

### `has_audio_data() -> bool`

Returns `true` if there is audio data available in the buffer.

```gdscript
if cef_texture.has_audio_data():
    print("Audio data available")
```

### `get_audio_buffer_size() -> int`

Returns the number of audio packets currently buffered.

```gdscript
var buffer_size = cef_texture.get_audio_buffer_size()
```

## Drag and Drop

These methods enable drag-and-drop operations between Godot and the CEF browser. For comprehensive documentation, see the [Drag and Drop](./drag-and-drop.md) page.

### `drag_enter(file_paths: Array[String], position: Vector2, allowed_ops: int)`

Notifies CEF that a drag operation has entered the browser area. Call this when handling Godot's `_can_drop_data()`.

```gdscript
func _can_drop_data(at_position: Vector2, data) -> bool:
    if data is Array:
        cef_texture.drag_enter(data, at_position, DragOperation.COPY)
        return true
    return false
```

### `drag_over(position: Vector2, allowed_ops: int)`

Updates the drag position as it moves over the browser. Call this repeatedly during drag operations.

```gdscript
cef_texture.drag_over(mouse_position, DragOperation.COPY)
```

### `drag_leave()`

Notifies CEF that a drag has left the browser area without dropping.

```gdscript
cef_texture.drag_leave()
```

### `drag_drop(position: Vector2)`

Completes the drag operation and drops the data at the specified position.

```gdscript
func _drop_data(at_position: Vector2, data):
    cef_texture.drag_drop(at_position)
```

### `drag_source_ended(position: Vector2, operation: int)`

Notifies CEF that a browser-initiated drag has ended. Call this when handling drops from the browser into your game.

```gdscript
cef_texture.drag_source_ended(drop_position, DragOperation.COPY)
```

### `drag_source_system_ended()`

Notifies CEF that the system drag operation has ended. Call this for cleanup after browser-initiated drags.

```gdscript
cef_texture.drag_source_system_ended()
```

### `is_dragging_from_browser() -> bool`

Returns `true` if a drag operation initiated from the browser is currently active.

```gdscript
if cef_texture.is_dragging_from_browser():
    print("Browser drag in progress")
```

### `is_drag_over() -> bool`

Returns `true` if a drag operation is currently over the CefTexture.

```gdscript
if cef_texture.is_drag_over():
    print("Drag is over browser area")
```

