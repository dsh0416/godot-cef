# 方法

`CefTexture` 提供一组方法，用于控制浏览器行为并与网页内容交互。

## 导航

### `go_back()`

在浏览器历史记录中后退。

```gdscript
cef_texture.go_back()
```

### `go_forward()`

在浏览器历史记录中前进。

```gdscript
cef_texture.go_forward()
```

### `can_go_back() -> bool`

如果浏览器可以后退，返回 `true`。

```gdscript
if cef_texture.can_go_back():
    cef_texture.go_back()
```

### `can_go_forward() -> bool`

如果浏览器可以前进，返回 `true`。

```gdscript
if cef_texture.can_go_forward():
    cef_texture.go_forward()
```

### `reload()`

重新加载当前页面。

```gdscript
cef_texture.reload()
```

### `reload_ignore_cache()`

重新加载当前页面，忽略任何缓存数据。

```gdscript
cef_texture.reload_ignore_cache()
```

### `stop_loading()`

停止加载当前页面。

```gdscript
cef_texture.stop_loading()
```

### `is_loading() -> bool`

如果浏览器当前正在加载页面，返回 `true`。

```gdscript
if cef_texture.is_loading():
    print("页面仍在加载中...")
```

## JavaScript 执行

### `eval(code: String)`

在浏览器主 Frame（main frame）中执行 JavaScript 代码。

```gdscript
# 执行 JavaScript
cef_texture.eval("document.body.style.backgroundColor = 'red'")

# 调用 JavaScript 函数
cef_texture.eval("updateScore(100)")

# 与 DOM 交互
cef_texture.eval("document.getElementById('player-name').innerText = 'Player1'")
```

## IPC（进程间通信）

### `send_ipc_message(message: String)`

从 Godot 向 JavaScript 发送消息。网页端如果注册了 `window.onIpcMessage(msg)` 回调，就会收到该消息。

```gdscript
# 发送简单字符串消息
cef_texture.send_ipc_message("Hello from Godot!")

# 使用 Dictionary 发送结构化数据作为 JSON
var payload := {"action": "update", "value": 42}
cef_texture.send_ipc_message(JSON.stringify(payload))
```

网页端 JavaScript（在 CEF 浏览器中运行）：

```javascript
// 注册回调以接收来自 Godot 的消息
window.onIpcMessage = function(msg) {
    console.log("从 Godot 收到:", msg);
    var data = JSON.parse(msg);
    // 处理消息...
};
```

### `send_ipc_binary_message(data: PackedByteArray)`

从 Godot 向 JavaScript 发送二进制数据。如果注册了 `window.onIpcBinaryMessage(arrayBuffer)` 回调，数据将作为 `ArrayBuffer` 传递。

使用原生 CEF 进程消息传递，零编码开销，可高效传输二进制数据（图像、音频、协议缓冲区等）。

```gdscript
# 发送原始二进制数据
var data := PackedByteArray([0x01, 0x02, 0x03, 0x04])
cef_texture.send_ipc_binary_message(data)

# 发送图像作为二进制
var image := Image.load_from_file("res://icon.png")
var png_data := image.save_png_to_buffer()
cef_texture.send_ipc_binary_message(png_data)

# 发送文件内容
var file := FileAccess.open("res://data.bin", FileAccess.READ)
var file_data := file.get_buffer(file.get_length())
cef_texture.send_ipc_binary_message(file_data)
```

在您的 JavaScript 中（在 CEF 浏览器中运行）：

```javascript
// 注册回调以接收来自 Godot 的二进制数据
window.onIpcBinaryMessage = function(arrayBuffer) {
    console.log("收到二进制数据:", arrayBuffer.byteLength, "字节");
    
    // 示例：处理为图像
    const blob = new Blob([arrayBuffer], { type: 'image/png' });
    const url = URL.createObjectURL(blob);
    document.getElementById('image').src = url;
    
    // 示例：处理为类型化数组
    const view = new Uint8Array(arrayBuffer);
    console.log("第一个字节:", view[0]);
};
```

## 缩放控制

### `set_zoom_level(level: float)`

设置浏览器的缩放级别。`0.0` 是默认值（100%）。正值放大，负值缩小。

```gdscript
cef_texture.set_zoom_level(1.0)   # 放大
cef_texture.set_zoom_level(-1.0)  # 缩小
cef_texture.set_zoom_level(0.0)   # 重置为默认
```

### `get_zoom_level() -> float`

返回当前缩放级别。

```gdscript
var zoom = cef_texture.get_zoom_level()
print("当前缩放: ", zoom)
```

## 音频控制

### `set_audio_muted(muted: bool)`

静音或取消静音浏览器音频。

```gdscript
cef_texture.set_audio_muted(true)   # 静音
cef_texture.set_audio_muted(false)  # 取消静音
```

### `is_audio_muted() -> bool`

如果浏览器音频已静音，返回 `true`。

```gdscript
if cef_texture.is_audio_muted():
    print("音频已静音")
```

## 音频捕获

这些方法可将浏览器音频通过 Godot 音频系统路由。详细文档请参见[音频捕获](./audio-capture.md)页面。

::: tip
在创建浏览器之前，必须在项目设置中启用音频捕获（`godot_cef/audio/enable_audio_capture`）。
:::

### `is_audio_capture_enabled() -> bool`

如果项目设置中启用了音频捕获模式，返回 `true`。

```gdscript
if cef_texture.is_audio_capture_enabled():
    print("音频捕获已启用")
```

### `create_audio_stream() -> AudioStreamGenerator`

创建并返回一个配置了正确采样率的 `AudioStreamGenerator`。

```gdscript
var audio_stream = cef_texture.create_audio_stream()
audio_player.stream = audio_stream
audio_player.play()
```

### `push_audio_to_playback(playback: AudioStreamGeneratorPlayback) -> int`

将 CEF 缓冲的音频数据推送到给定的播放器。返回推送的帧数。在 `_process()` 中每帧调用此方法。

```gdscript
func _process(_delta):
    var playback = audio_player.get_stream_playback()
    if playback:
        cef_texture.push_audio_to_playback(playback)
```

### `has_audio_data() -> bool`

如果缓冲区中有可用的音频数据，返回 `true`。

```gdscript
if cef_texture.has_audio_data():
    print("有可用的音频数据")
```

### `get_audio_buffer_size() -> int`

返回当前缓冲的音频数据包数量。

```gdscript
var buffer_size = cef_texture.get_audio_buffer_size()
```

## 拖放

这些方法可在 Godot 和 CEF 浏览器之间进行拖放操作。详细文档请参见[拖放](./drag-and-drop.md)页面。

### `drag_enter(file_paths: Array[String], position: Vector2, allowed_ops: int)`

通知 CEF 拖动操作已进入浏览器区域。在处理 Godot 的 `_can_drop_data()` 时调用此方法。

```gdscript
func _can_drop_data(at_position: Vector2, data) -> bool:
    if data is Array:
        cef_texture.drag_enter(data, at_position, DragOperation.COPY)
        return true
    return false
```

### `drag_over(position: Vector2, allowed_ops: int)`

在拖动移动到浏览器上方时更新拖动位置。在拖动操作期间重复调用此方法。

```gdscript
cef_texture.drag_over(mouse_position, DragOperation.COPY)
```

### `drag_leave()`

通知 CEF 拖动已离开浏览器区域但未放下。

```gdscript
cef_texture.drag_leave()
```

### `drag_drop(position: Vector2)`

完成拖动操作并在指定位置放下数据。

```gdscript
func _drop_data(at_position: Vector2, data):
    cef_texture.drag_drop(at_position)
```

### `drag_source_ended(position: Vector2, operation: int)`

通知 CEF 浏览器发起的拖动已结束。在处理从浏览器拖放到游戏中时调用此方法。

```gdscript
cef_texture.drag_source_ended(drop_position, DragOperation.COPY)
```

### `drag_source_system_ended()`

通知 CEF 系统拖动操作已结束。在浏览器发起的拖动后用于清理。

```gdscript
cef_texture.drag_source_system_ended()
```

### `is_dragging_from_browser() -> bool`

如果当前有从浏览器发起的拖动操作正在进行，返回 `true`。

```gdscript
if cef_texture.is_dragging_from_browser():
    print("浏览器拖动正在进行")
```

### `is_drag_over() -> bool`

如果当前有拖动操作在 CefTexture 上方，返回 `true`。

```gdscript
if cef_texture.is_drag_over():
    print("拖动在浏览器区域上方")
```

