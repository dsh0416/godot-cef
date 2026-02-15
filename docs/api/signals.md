# Signals

The `CefTexture` node emits various signals to notify your game about browser events and state changes.

## `ipc_message(message: String)`

Emitted when JavaScript sends a message to Godot via the `sendIpcMessage` function. Use this for bidirectional communication between your web UI and game logic.

```gdscript
func _ready():
    cef_texture.ipc_message.connect(_on_ipc_message)

func _on_ipc_message(message: String):
    print("Received from web: ", message)
    var data = JSON.parse_string(message)
    # Handle the message...
```

In your JavaScript (running in the CEF browser):

```javascript
// Send a message to Godot
window.sendIpcMessage("button_clicked");

// Send structured data as JSON
window.sendIpcMessage(JSON.stringify({ action: "purchase", item_id: 42 }));
```

## `ipc_binary_message(data: PackedByteArray)`

Emitted when JavaScript sends binary data to Godot via the `sendIpcBinaryMessage` function. Use this for efficient binary data transfer without Base64 encoding overhead.

```gdscript
func _ready():
    cef_texture.ipc_binary_message.connect(_on_ipc_binary_message)

func _on_ipc_binary_message(data: PackedByteArray):
    print("Received binary data: ", data.size(), " bytes")
    # Process binary data (e.g., protobuf, msgpack, raw bytes)
    var image = Image.new()
    image.load_png_from_buffer(data)
```

In your JavaScript (running in the CEF browser):

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

## `ipc_data_message(data: Variant)`

Emitted when JavaScript sends typed data to Godot via `sendIpcData(value)`. The payload is transported through the CBOR lane and converted back into a Godot `Variant`.

```gdscript
func _ready():
    cef_texture.ipc_data_message.connect(_on_ipc_data_message)

func _on_ipc_data_message(data: Variant):
    print("Typed IPC data type: ", typeof(data))
    if data is Dictionary and data.has("type"):
        print("Event type: ", data["type"])
```

In your JavaScript (running in the CEF browser):

```javascript
window.sendIpcData({
    type: "inventory_update",
    items: ["potion", "elixir"]
});
```

## JavaScript Listener APIs

In addition to legacy callback globals, renderer-side listener objects are available and support multiple subscribers:

```javascript
const handler = (msg) => console.log("IPC:", msg);
window.ipcMessage.addListener(handler);
console.log(window.ipcMessage.hasListener(handler)); // true
window.ipcMessage.removeListener(handler);
```

Available listener objects:
- `window.ipcMessage`
- `window.ipcBinaryMessage`
- `window.ipcDataMessage`

## `url_changed(url: String)`

Emitted when the browser navigates to a new URL. This fires for user-initiated navigation (clicking links), JavaScript navigation, redirects, and programmatic `load_url()` calls. Useful for injecting scripts or tracking navigation.

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

Emitted when the page title changes. Useful for updating window titles or UI elements.

```gdscript
func _ready():
    cef_texture.title_changed.connect(_on_title_changed)

func _on_title_changed(title: String):
    print("Page title: ", title)
    $TitleLabel.text = title
```

## `load_started(url: String)`

Emitted when the browser starts loading a page.

```gdscript
func _ready():
    cef_texture.load_started.connect(_on_load_started)

func _on_load_started(url: String):
    print("Loading: ", url)
    $LoadingSpinner.visible = true
```

## `load_finished(url: String, http_status_code: int)`

Emitted when the browser finishes loading a page. The `http_status_code` contains the HTTP response status (e.g., 200 for success, 404 for not found).

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

Emitted when a page load error occurs (e.g., network error, invalid URL).

```gdscript
func _ready():
    cef_texture.load_error.connect(_on_load_error)

func _on_load_error(url: String, error_code: int, error_text: String):
    print("Failed to load: ", url)
    print("Error ", error_code, ": ", error_text)
    # Show error page or retry
```

## `console_message(level: int, message: String, source: String, line: int)`

Emitted when JavaScript logs a message to the browser console (e.g., `console.log()`, `console.warn()`, `console.error()`). Useful for debugging web content or capturing JavaScript errors.

**Parameters:**
- `level`: Log severity level (0=debug, 1=info, 2=warning, 3=error, 4=fatal)
- `message`: The console message text
- `source`: The source file URL where the message originated
- `line`: The line number in the source file

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

Emitted when the user starts dragging content from the web page (e.g., an image, link, or selected text). Use this to handle browser-initiated drags in your game.

**Parameters:**
- `drag_data`: A `DragDataInfo` object containing information about what's being dragged
- `position`: The starting position of the drag in local coordinates
- `allowed_ops`: Bitmask of allowed drag operations (see `DragOperation` constants)

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

Emitted when the drag cursor should change based on the current drop target. Use this to update visual feedback during drag operations.

**Parameters:**
- `operation`: The drag operation that would occur if dropped (see `DragOperation` constants)

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

Emitted when a drag operation enters the CefTexture from an external source.

**Parameters:**
- `drag_data`: A `DragDataInfo` object containing information about what's being dragged
- `mask`: Bitmask of allowed operations

```gdscript
func _ready():
    cef_texture.drag_entered.connect(_on_drag_entered)

func _on_drag_entered(drag_data: DragDataInfo, mask: int):
    print("Drag entered browser area")
```

::: tip
For comprehensive drag-and-drop documentation including methods for handling Godot → CEF drags, see the [Drag and Drop](./drag-and-drop.md) page.
:::

## `download_requested(download_info: DownloadRequestInfo)`

Emitted when a download is requested (e.g., user clicks a download link). The download does **not** start automatically; you must handle this signal to decide what to do with the download.

**Parameters:**
- `download_info`: A `DownloadRequestInfo` object containing:
  - `id: int` - Unique identifier for this download
  - `url: String` - The URL being downloaded
  - `original_url: String` - The original URL before any redirects
  - `suggested_file_name: String` - Suggested file name from the server
  - `mime_type: String` - MIME type of the download
  - `total_bytes: int` - Total size in bytes, or -1 if unknown

```gdscript
func _ready():
    cef_texture.download_requested.connect(_on_download_requested)

func _on_download_requested(download_info: DownloadRequestInfo):
    print("Download: %s (%d bytes)" % [download_info.suggested_file_name, download_info.total_bytes])
```

::: tip
Downloads don't start automatically—handle this signal to show a confirmation dialog or save the file.
:::

## `download_updated(download_info: DownloadUpdateInfo)`

Emitted when a download's progress changes or completes. Use this to track download progress and handle completion.

**Parameters:**
- `download_info`: A `DownloadUpdateInfo` object containing:
  - `id: int` - Unique identifier for this download (matches `download_requested`)
  - `url: String` - The URL being downloaded
  - `full_path: String` - Full path where the file is being saved
  - `received_bytes: int` - Bytes received so far
  - `total_bytes: int` - Total size in bytes, or -1 if unknown
  - `current_speed: int` - Current download speed in bytes per second
  - `percent_complete: int` - Percentage complete (0-100), or -1 if unknown
  - `is_in_progress: bool` - Whether the download is still in progress
  - `is_complete: bool` - Whether the download completed successfully
  - `is_canceled: bool` - Whether the download was canceled

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

Emitted when the browser tries to open a popup window (e.g., `window.open()`, links with `target="_blank"`). This signal is only emitted when `popup_policy` is set to `2` (SIGNAL_ONLY).

**Parameters:**
- `url`: The URL the popup wants to navigate to
- `disposition`: How CEF requested the window to be opened. Values match CEF's `cef_window_open_disposition_t`:
  - `0` = Current tab
  - `1` = Singleton tab
  - `2` = New foreground tab
  - `3` = New background tab
  - `4` = New popup
  - `5` = New window
  - `6` = Save to disk
  - `7` = Off the record
  - `8` = Ignore action
- `user_gesture`: Whether the popup was triggered by a user gesture (click, keyboard shortcut, etc.)

```gdscript
func _ready():
    cef_texture.popup_policy = 2  # SIGNAL_ONLY
    cef_texture.popup_requested.connect(_on_popup_requested)

func _on_popup_requested(url: String, disposition: int, user_gesture: bool):
    print("Popup requested: ", url, " (gesture: ", user_gesture, ")")
    if user_gesture:
        # Navigate the current browser to the URL
        cef_texture.url = url
    else:
        # Block non-user-initiated popups (likely ads)
        print("Blocked popup: ", url)
```

::: tip Popup Policy
Use the `popup_policy` property to control popup behavior:
- `0` (BLOCK): Suppress all popups silently (default, safest)
- `1` (REDIRECT): Automatically navigate the current browser to the popup URL
- `2` (SIGNAL_ONLY): Emit this signal and let your GDScript decide what to do
:::

## `permission_requested(permission_type: String, url: String, request_id: int)`

Emitted when a web page requests a permission (for example camera, microphone, geolocation, clipboard, or notifications), and `godot_cef/security/default_permission_policy` is set to `2` (SIGNAL).

**Parameters:**
- `permission_type`: Requested permission type (for example `camera`, `microphone`, `geolocation`, `clipboard`, `notifications`)
- `url`: Requesting origin URL
- `request_id`: Unique request ID used by `grant_permission()` / `deny_permission()`

When a single browser prompt includes multiple permissions (for example camera + microphone), this signal is emitted once per permission type, each with a distinct `request_id`.

```gdscript
func _ready():
    cef_texture.permission_requested.connect(_on_permission_requested)

func _on_permission_requested(permission_type: String, url: String, request_id: int):
    print("Permission requested: ", permission_type, " from ", url)
    var allow = permission_type in ["microphone", "camera"] and url.begins_with("https://trusted.example")
    if allow:
        cef_texture.grant_permission(request_id)
    else:
        cef_texture.deny_permission(request_id)
```

## `find_result(count: int, active_index: int, final_update: bool)`

Emitted while a find-in-page search is running.

**Parameters:**
- `count`: Total number of matches found
- `active_index`: Index of the currently highlighted match
- `final_update`: `true` when this callback contains the final result for the current query

```gdscript
func _ready():
    cef_texture.find_result.connect(_on_find_result)

func _on_find_result(count: int, active_index: int, final_update: bool):
    if final_update:
        $FindLabel.text = "%d matches (%d selected)" % [count, active_index]
```

## `render_process_terminated(status: int, error_message: String)`

Emitted when the browser's render process terminates unexpectedly (crash, killed by OS, etc.).

**Parameters:**
- `status`: Termination status code from CEF
- `error_message`: Description of the termination reason

```gdscript
func _ready():
    cef_texture.render_process_terminated.connect(_on_render_process_terminated)

func _on_render_process_terminated(status: int, error_message: String):
    print("Render process terminated: ", error_message, " (status: ", status, ")")
    # Optionally reload the page
    cef_texture.reload()
```

## `cookies_received(cookies: Array[CookieInfo])`

Emitted when `get_all_cookies()` or `get_cookies()` completes. Contains an array of `CookieInfo` objects with the retrieved cookie data.

Each `CookieInfo` object has the following properties:

| Property | Type | Description |
|----------|------|-------------|
| `name` | `String` | Cookie name |
| `value` | `String` | Cookie value |
| `domain` | `String` | Cookie domain (e.g., `.example.com`) |
| `path` | `String` | Cookie path (e.g., `/`) |
| `secure` | `bool` | Whether the cookie requires HTTPS |
| `httponly` | `bool` | Whether the cookie is HTTP-only (not accessible via JavaScript) |
| `same_site` | `int` | SameSite policy: `0` = Unspecified, `1` = None, `2` = Lax, `3` = Strict |
| `has_expires` | `bool` | Whether the cookie has an expiration date (session cookies have `false`) |

```gdscript
func _ready():
    cef_texture.cookies_received.connect(_on_cookies_received)

func _on_cookies_received(cookies: Array[CookieInfo]):
    for cookie in cookies:
        print("Cookie: ", cookie.name, " = ", cookie.value,
              " (domain: ", cookie.domain, ", secure: ", cookie.secure, ")")

# Retrieve all cookies
cef_texture.get_all_cookies()

# Retrieve cookies for a specific URL
cef_texture.get_cookies("https://example.com", true)
```

## `cookie_set(success: bool)`

Emitted when a `set_cookie()` call completes.

**Parameters:**
- `success`: Whether the cookie was set successfully

```gdscript
func _ready():
    cef_texture.cookie_set.connect(_on_cookie_set)

func _on_cookie_set(success: bool):
    if success:
        print("Cookie set successfully")
    else:
        print("Failed to set cookie")
```

## `cookies_deleted(num_deleted: int)`

Emitted when `delete_cookies()` or `clear_cookies()` completes.

**Parameters:**
- `num_deleted`: The number of cookies that were deleted

```gdscript
func _ready():
    cef_texture.cookies_deleted.connect(_on_cookies_deleted)

func _on_cookies_deleted(num_deleted: int):
    print("Deleted ", num_deleted, " cookies")
```

## `cookies_flushed()`

Emitted when `flush_cookies()` completes. The cookie store has been written to disk.

```gdscript
func _ready():
    cef_texture.cookies_flushed.connect(_on_cookies_flushed)

func _on_cookies_flushed():
    print("Cookie store flushed to disk")
```

## Signal Usage Patterns

### Loading State Management

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

### IPC Communication

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
