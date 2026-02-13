# 信号

`CefTexture` 会发出一系列信号，用于通知游戏侧浏览器事件与状态变化。

## `ipc_message(message: String)`

当网页端通过 `sendIpcMessage` 向 Godot 发送消息时发出。用于网页 UI 与游戏逻辑之间的双向通信（IPC）。

```gdscript
func _ready():
    cef_texture.ipc_message.connect(_on_ipc_message)

func _on_ipc_message(message: String):
    print("Received from web: ", message)
    var data = JSON.parse_string(message)
    # Handle the message...
```

网页端 JavaScript（在 CEF 浏览器中运行）：

```javascript
// Send a message to Godot
window.sendIpcMessage("button_clicked");

// Send structured data as JSON
window.sendIpcMessage(JSON.stringify({ action: "purchase", item_id: 42 }));
```

## `ipc_binary_message(data: PackedByteArray)`

当 JavaScript 通过 `sendIpcBinaryMessage` 函数向 Godot 发送二进制数据时发出。用于高效的二进制数据传输，无需 Base64 编码开销。

```gdscript
func _ready():
    cef_texture.ipc_binary_message.connect(_on_ipc_binary_message)

func _on_ipc_binary_message(data: PackedByteArray):
    print("Received binary data: ", data.size(), " bytes")
    # Process binary data (e.g., protobuf, msgpack, raw bytes)
    var image = Image.new()
    image.load_png_from_buffer(data)
```

在您的 JavaScript 中（在 CEF 浏览器中运行）：

```javascript
// Send binary data to Godot
const buffer = new ArrayBuffer(8);
const view = new Uint8Array(buffer);
view.set([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]); // PNG header
window.sendIpcBinaryMessage(buffer);

// Send a Uint8Array (will use its underlying ArrayBuffer)
const data = new Uint8Array([1, 2, 3, 4, 5]);
window.sendIpcBinaryMessage(data.buffer);
```

## `url_changed(url: String)`

当浏览器导航到新 URL 时发出。这会在用户发起的导航（点击链接）、JavaScript 导航、重定向和程序化 `load_url()` 调用时触发。用于注入脚本或跟踪导航。

```gdscript
func _ready():
    cef_texture.url_changed.connect(_on_url_changed)

func _on_url_changed(url: String):
    print("Navigated to: ", url)
    # Inject data based on the current page
    if "game-ui" in url:
        cef_texture.eval("window.playerData = %s" % JSON.stringify(player_data))
```

## `title_changed(title: String)`

当页面标题更改时发出。用于更新窗口标题或 UI 元素。

```gdscript
func _ready():
    cef_texture.title_changed.connect(_on_title_changed)

func _on_title_changed(title: String):
    print("Page title: ", title)
    $TitleLabel.text = title
```

## `load_started(url: String)`

当浏览器开始加载页面时发出。

```gdscript
func _ready():
    cef_texture.load_started.connect(_on_load_started)

func _on_load_started(url: String):
    print("Loading: ", url)
    $LoadingSpinner.visible = true
```

## `load_finished(url: String, http_status_code: int)`

当浏览器完成加载页面时发出。`http_status_code` 包含 HTTP 响应状态（例如 200 表示成功，404 表示未找到）。

```gdscript
func _ready():
    cef_texture.load_finished.connect(_on_load_finished)

func _on_load_finished(url: String, http_status_code: int):
    print("Loaded: ", url, " (status: ", http_status_code, ")")
    $LoadingSpinner.visible = false
    if http_status_code != 200:
        print("Warning: Page returned status ", http_status_code)
```

## `load_error(url: String, error_code: int, error_text: String)`

当页面加载发生错误时发出（例如网络错误、无效 URL）。

```gdscript
func _ready():
    cef_texture.load_error.connect(_on_load_error)

func _on_load_error(url: String, error_code: int, error_text: String):
    print("Failed to load: ", url)
    print("Error ", error_code, ": ", error_text)
    # Show error page or retry
```

## `console_message(level: int, message: String, source: String, line: int)`

当 JavaScript 向浏览器控制台记录消息时发出（例如 `console.log()`、`console.warn()`、`console.error()`）。用于调试网页内容或捕获 JavaScript 错误。

**参数：**
- `level`：日志严重级别（0=调试, 1=信息, 2=警告, 3=错误, 4=致命）
- `message`：控制台消息文本
- `source`：消息来源的源文件 URL
- `line`：源文件中的行号

```gdscript
func _ready():
    cef_texture.console_message.connect(_on_console_message)

func _on_console_message(level: int, message: String, source: String, line: int):
    var level_names = ["DEBUG", "INFO", "WARNING", "ERROR", "FATAL"]
    var level_name = level_names[level] if level < level_names.size() else "UNKNOWN"
    print("[%s] %s (%s:%d)" % [level_name, message, source, line])
    
    # Capture JavaScript errors for debugging
    if level >= 3:  # ERROR or FATAL
        push_error("JS Error: %s at %s:%d" % [message, source, line])
```

## `drag_started(drag_data: DragDataInfo, position: Vector2, allowed_ops: int)`

当用户开始从网页拖动内容时发出（例如图像、链接或选中的文本）。用于在游戏中处理浏览器发起的拖动。

**参数：**
- `drag_data`：包含正在拖动内容信息的 `DragDataInfo` 对象
- `position`：本地坐标中拖动的起始位置
- `allowed_ops`：允许的拖动操作的位掩码（参见 `DragOperation` 常量）

```gdscript
func _ready():
    cef_texture.drag_started.connect(_on_drag_started)

func _on_drag_started(drag_data: DragDataInfo, position: Vector2, allowed_ops: int):
    if drag_data.is_link:
        print("Dragging link: ", drag_data.link_url)
        # Start custom drag handling in your game
    elif drag_data.is_fragment:
        print("Dragging text: ", drag_data.fragment_text)
```

## `drag_cursor_updated(operation: int)`

当拖动光标应根据当前放置目标更改时发出。用于在拖动操作期间更新视觉反馈。

**参数：**
- `operation`：如果放下将发生的拖动操作（参见 `DragOperation` 常量）

```gdscript
func _ready():
    cef_texture.drag_cursor_updated.connect(_on_drag_cursor_updated)

func _on_drag_cursor_updated(operation: int):
    match operation:
        DragOperation.COPY:
            Input.set_default_cursor_shape(Input.CURSOR_DRAG)
        DragOperation.NONE:
            Input.set_default_cursor_shape(Input.CURSOR_FORBIDDEN)
```

## `drag_entered(drag_data: DragDataInfo, mask: int)`

当拖动操作从外部源进入 CefTexture 时发出。

**参数：**
- `drag_data`：包含正在拖动内容信息的 `DragDataInfo` 对象
- `mask`：允许操作的位掩码

```gdscript
func _ready():
    cef_texture.drag_entered.connect(_on_drag_entered)

func _on_drag_entered(drag_data: DragDataInfo, mask: int):
    print("Drag entered browser area")
```

::: tip
有关包括处理 Godot → CEF 拖动方法的完整拖放文档，请参见[拖放](./drag-and-drop.md)页面。
:::

## `download_requested(download_info: DownloadRequestInfo)`

当请求下载时发出（例如用户点击下载链接）。下载**不会**自动开始；您必须处理此信号来决定如何处理下载。

**参数：**
- `download_info`：包含以下内容的 `DownloadRequestInfo` 对象：
  - `id: int` - 此下载的唯一标识符
  - `url: String` - 正在下载的 URL
  - `original_url: String` - 任何重定向之前的原始 URL
  - `suggested_file_name: String` - 服务器建议的文件名
  - `mime_type: String` - 下载的 MIME 类型
  - `total_bytes: int` - 总大小（字节），如果未知则为 -1

```gdscript
func _ready():
    cef_texture.download_requested.connect(_on_download_requested)

func _on_download_requested(download_info: DownloadRequestInfo):
    print("Download: %s (%d bytes)" % [download_info.suggested_file_name, download_info.total_bytes])
```

::: tip
下载不会自动开始——处理此信号以显示确认对话框或保存文件。
:::

## `download_updated(download_info: DownloadUpdateInfo)`

当下载进度更改或完成时发出。用于跟踪下载进度和处理完成。

**参数：**
- `download_info`：包含以下内容的 `DownloadUpdateInfo` 对象：
  - `id: int` - 此下载的唯一标识符（与 `download_requested` 匹配）
  - `url: String` - 正在下载的 URL
  - `full_path: String` - 文件保存的完整路径
  - `received_bytes: int` - 目前已接收的字节数
  - `total_bytes: int` - 总大小（字节），如果未知则为 -1
  - `current_speed: int` - 当前下载速度（字节/秒）
  - `percent_complete: int` - 完成百分比（0-100），如果未知则为 -1
  - `is_in_progress: bool` - 下载是否仍在进行中
  - `is_complete: bool` - 下载是否成功完成
  - `is_canceled: bool` - 下载是否已取消

```gdscript
func _ready():
    cef_texture.download_updated.connect(_on_download_updated)

func _on_download_updated(download_info: DownloadUpdateInfo):
    if download_info.is_complete:
        print("Download complete: ", download_info.full_path)
    elif download_info.is_canceled:
        print("Download canceled: ", download_info.url)
    elif download_info.is_in_progress:
        var percent = download_info.percent_complete
        var speed_kb = download_info.current_speed / 1024.0
        print("Downloading: %d%% (%.1f KB/s)" % [percent, speed_kb])
```

## `popup_requested(url: String, disposition: int, user_gesture: bool)`

当浏览器尝试打开弹出窗口时触发（例如 `window.open()`、`target="_blank"` 链接）。此信号仅在 `popup_policy` 设为 `2`（SIGNAL_ONLY）时才会触发。

**参数：**
- `url`：弹出窗口要导航到的 URL
- `disposition`：CEF 请求打开窗口的方式。值对应 CEF 的 `cef_window_open_disposition_t`：
  - `0` = 当前标签页
  - `1` = 单例标签页
  - `2` = 新前台标签页
  - `3` = 新后台标签页
  - `4` = 新弹出窗口
  - `5` = 新窗口
  - `6` = 保存到磁盘
  - `7` = 无痕模式
  - `8` = 忽略操作
- `user_gesture`：弹出是否由用户手势触发（点击、键盘快捷键等）

```gdscript
func _ready():
    cef_texture.popup_policy = 2  # SIGNAL_ONLY
    cef_texture.popup_requested.connect(_on_popup_requested)

func _on_popup_requested(url: String, disposition: int, user_gesture: bool):
    print("弹出请求: ", url, " (手势: ", user_gesture, ")")
    if user_gesture:
        # 在当前浏览器中导航到该 URL
        cef_texture.url = url
    else:
        # 阻止非用户触发的弹出（可能是广告）
        print("已阻止弹出: ", url)
```

::: tip 弹出策略
使用 `popup_policy` 属性控制弹出行为：
- `0`（BLOCK）：静默阻止所有弹出（默认，最安全）
- `1`（REDIRECT）：自动将当前浏览器导航到弹出 URL
- `2`（SIGNAL_ONLY）：触发此信号，由 GDScript 决定如何处理
:::

## `render_process_terminated(status: int, error_message: String)`

当浏览器的渲染进程意外终止时触发（崩溃、被操作系统终止等）。

**参数：**
- `status`：来自 CEF 的终止状态码
- `error_message`：终止原因描述

```gdscript
func _ready():
    cef_texture.render_process_terminated.connect(_on_render_process_terminated)

func _on_render_process_terminated(status: int, error_message: String):
    print("渲染进程终止: ", error_message, " (状态: ", status, ")")
    # 可选择重新加载页面
    cef_texture.reload()
```

## `cookies_received(cookies: Array[CookieInfo])`

当 `get_all_cookies()` 或 `get_cookies()` 完成时触发。包含检索到的 Cookie 数据的 `CookieInfo` 对象数组。

每个 `CookieInfo` 对象具有以下属性：

| 属性 | 类型 | 描述 |
|------|------|------|
| `name` | `String` | Cookie 名称 |
| `value` | `String` | Cookie 值 |
| `domain` | `String` | Cookie 域（如 `.example.com`） |
| `path` | `String` | Cookie 路径（如 `/`） |
| `secure` | `bool` | 是否仅通过 HTTPS 发送 |
| `httponly` | `bool` | 是否为 HTTP-only（JavaScript 无法访问） |
| `same_site` | `int` | SameSite 策略：`0` = 未指定, `1` = None, `2` = Lax, `3` = Strict |
| `has_expires` | `bool` | 是否有过期时间（会话 Cookie 为 `false`） |

```gdscript
func _ready():
    cef_texture.cookies_received.connect(_on_cookies_received)

func _on_cookies_received(cookies: Array[CookieInfo]):
    for cookie in cookies:
        print("Cookie: ", cookie.name, " = ", cookie.value,
              " (domain: ", cookie.domain, ", secure: ", cookie.secure, ")")

# 获取所有 Cookie
cef_texture.get_all_cookies()

# 获取特定 URL 的 Cookie
cef_texture.get_cookies("https://example.com", true)
```

## `cookie_set(success: bool)`

当 `set_cookie()` 调用完成时触发。

**参数：**
- `success`：Cookie 是否设置成功

```gdscript
func _ready():
    cef_texture.cookie_set.connect(_on_cookie_set)

func _on_cookie_set(success: bool):
    if success:
        print("Cookie 设置成功")
    else:
        print("Cookie 设置失败")
```

## `cookies_deleted(num_deleted: int)`

当 `delete_cookies()` 或 `clear_cookies()` 完成时触发。

**参数：**
- `num_deleted`：已删除的 Cookie 数量

```gdscript
func _ready():
    cef_texture.cookies_deleted.connect(_on_cookies_deleted)

func _on_cookies_deleted(num_deleted: int):
    print("已删除 ", num_deleted, " 个 Cookie")
```

## `cookies_flushed()`

当 `flush_cookies()` 完成时触发。Cookie 存储已写入磁盘。

```gdscript
func _ready():
    cef_texture.cookies_flushed.connect(_on_cookies_flushed)

func _on_cookies_flushed():
    print("Cookie 存储已刷新到磁盘")
```

## 信号使用模式

### 加载状态管理

```gdscript
extends Control

@onready var browser = $CefTexture
@onready var loading_indicator = $LoadingIndicator

func _ready():
    browser.load_started.connect(_on_load_started)
    browser.load_finished.connect(_on_load_finished)
    browser.load_error.connect(_on_load_error)

func _on_load_started(url: String):
    loading_indicator.visible = true
    print("Started loading: ", url)

func _on_load_finished(url: String, status: int):
    loading_indicator.visible = false
    if status == 200:
        print("Successfully loaded: ", url)
    else:
        print("Loaded with status: ", status)

func _on_load_error(url: String, error_code: int, error_text: String):
    loading_indicator.visible = false
    print("Failed to load ", url, ": ", error_text)
    # Could show error page or retry logic here
```

### IPC 通信

```gdscript
extends Node

@onready var browser = $CefTexture

func _ready():
    browser.ipc_message.connect(_handle_web_message)

func _handle_web_message(message: String):
    var data = JSON.parse_string(message)
    match data.get("type"):
        "player_action":
            _handle_player_action(data)
        "ui_event":
            _handle_ui_event(data)
        "game_state":
            _update_game_state(data)

# Send messages to web UI
func send_to_web_ui(action: String, payload: Dictionary):
    var message = {"type": action, "data": payload}
    browser.send_ipc_message(JSON.stringify(message))
```

