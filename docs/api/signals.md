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
