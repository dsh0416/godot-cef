# Drag and Drop

CefTexture supports bidirectional drag-and-drop operations between Godot and the embedded CEF browser. This allows you to:

- **Drag files or data from Godot into the browser** (e.g., dropping files onto a web page)
- **Handle drags initiated from the browser** via signals (e.g., dragging an image or link from a web page into your game)

## Concepts

### Drag Operations

Drag operations are represented by the `DragOperation` class constants:

| Constant | Value | Description |
|----------|-------|-------------|
| `NONE` | 0 | No operation allowed |
| `COPY` | 1 | Copy the dragged data |
| `LINK` | 2 | Create a link to the dragged data |
| `MOVE` | 16 | Move the dragged data |
| `EVERY` | MAX_INT | All operations allowed |

### DragDataInfo

When a drag event occurs, you receive a `DragDataInfo` object containing information about what's being dragged:

| Property | Type | Description |
|----------|------|-------------|
| `is_link` | `bool` | True if dragging a URL link |
| `is_file` | `bool` | True if dragging files |
| `is_fragment` | `bool` | True if dragging text/HTML content |
| `link_url` | `String` | The URL being dragged (if `is_link`) |
| `link_title` | `String` | The title of the link (if `is_link`) |
| `fragment_text` | `String` | Plain text content (if `is_fragment`) |
| `fragment_html` | `String` | HTML content (if `is_fragment`) |
| `file_names` | `Array[String]` | List of file paths (if `is_file`) |

## Godot → CEF Browser (Dropping Files Into Web Page)

To enable dropping files or data into the CEF browser, you need to call methods on `CefTexture` when handling Godot's drag-and-drop events.

### Methods

#### `drag_enter(file_paths: Array[String], position: Vector2, allowed_ops: int)`

Call when a drag enters the `CefTexture` area. This notifies CEF that a drag operation is starting.

```gdscript
func _can_drop_data(at_position: Vector2, data) -> bool:
    if data is Array:
        cef_texture.drag_enter(data, at_position, DragOperation.COPY)
        return true
    return false
```

#### `drag_over(position: Vector2, allowed_ops: int)`

Call repeatedly as the drag moves over the `CefTexture`. This updates the drag position for CEF.

```gdscript
func _process(delta):
    if is_dragging and cef_texture.is_drag_over():
        var mouse_pos = get_local_mouse_position()
        cef_texture.drag_over(mouse_pos, DragOperation.COPY)
```

#### `drag_leave()`

Call when the drag leaves the `CefTexture` area without dropping.

```gdscript
func _on_mouse_exited():
    if cef_texture.is_drag_over():
        cef_texture.drag_leave()
```

#### `drag_drop(position: Vector2)`

Call when the user releases the drag to drop the data onto the web page.

```gdscript
func _drop_data(at_position: Vector2, data):
    cef_texture.drag_drop(at_position)
```

### Complete Example: File Drop Zone

```gdscript
extends Control

@onready var cef_texture = $CefTexture

var is_dragging := false

func _ready():
    cef_texture.url = "https://example.com/upload"

func _can_drop_data(at_position: Vector2, data) -> bool:
    # Accept arrays of file paths
    if data is Array:
        cef_texture.drag_enter(data, at_position, DragOperation.COPY)
        is_dragging = true
        return true
    return false

func _drop_data(at_position: Vector2, data):
    cef_texture.drag_drop(at_position)
    is_dragging = false

func _notification(what):
    # Handle drag leaving the control
    if what == NOTIFICATION_DRAG_END and is_dragging:
        cef_texture.drag_leave()
        is_dragging = false
```

## CEF Browser → Godot (Handling Browser-Initiated Drags)

When users start dragging content from the web page (e.g., an image, link, or selected text), CefTexture emits signals that you can connect to and handle in your game.

### Signals

#### `drag_started(drag_data: DragDataInfo, position: Vector2, allowed_ops: int)`

Emitted when the user starts dragging content from the web page.

```gdscript
func _ready():
    cef_texture.drag_started.connect(_on_drag_started)

func _on_drag_started(drag_data: DragDataInfo, position: Vector2, allowed_ops: int):
    print("Drag started at: ", position)
    
    if drag_data.is_link:
        print("Dragging link: ", drag_data.link_url)
        # Create a preview for the dragged link
        start_custom_drag(drag_data.link_url, drag_data.link_title)
    elif drag_data.is_file:
        print("Dragging files: ", drag_data.file_names)
    elif drag_data.is_fragment:
        print("Dragging text: ", drag_data.fragment_text)
```

#### `drag_cursor_updated(operation: int)`

Emitted when the drag cursor visual should change based on the allowed operations at the current position.

```gdscript
func _ready():
    cef_texture.drag_cursor_updated.connect(_on_drag_cursor_updated)

func _on_drag_cursor_updated(operation: int):
    match operation:
        DragOperation.COPY:
            Input.set_default_cursor_shape(Input.CURSOR_DRAG)
        DragOperation.NONE:
            Input.set_default_cursor_shape(Input.CURSOR_FORBIDDEN)
        _:
            Input.set_default_cursor_shape(Input.CURSOR_ARROW)
```

#### `drag_entered(drag_data: DragDataInfo, mask: int)`

Emitted when a drag enters the CefTexture from an external source (via the CEF drag handler).

```gdscript
func _ready():
    cef_texture.drag_entered.connect(_on_drag_entered)

func _on_drag_entered(drag_data: DragDataInfo, mask: int):
    print("External drag entered with ops mask: ", mask)
```

### Notifying CEF When Browser Drag Ends

When a drag that started from the browser ends (either dropped somewhere or cancelled), you should notify CEF:

#### `drag_source_ended(position: Vector2, operation: int)`

Call when a browser-initiated drag ends with a specific result.

```gdscript
func _on_drop_completed(drop_position: Vector2, was_accepted: bool):
    if cef_texture.is_dragging_from_browser():
        var op = DragOperation.COPY if was_accepted else DragOperation.NONE
        cef_texture.drag_source_ended(drop_position, op)
```

#### `drag_source_system_ended()`

Call when the system drag operation ends (cleanup).

```gdscript
func _notification(what):
    if what == NOTIFICATION_DRAG_END:
        if cef_texture.is_dragging_from_browser():
            cef_texture.drag_source_system_ended()
```

### Query Methods

#### `is_dragging_from_browser() -> bool`

Returns `true` if a drag operation is currently active that was initiated from the browser.

#### `is_drag_over() -> bool`

Returns `true` if a drag operation is currently over the CefTexture.

## Complete Example: Handling Browser Drags

```gdscript
extends Control

@onready var cef_texture = $CefTexture
@onready var inventory = $Inventory  # Your game's inventory system

var browser_drag_data: DragDataInfo = null

func _ready():
    cef_texture.url = "https://game-shop.example.com"
    
    # Connect to drag signals
    cef_texture.drag_started.connect(_on_drag_started)
    cef_texture.drag_cursor_updated.connect(_on_drag_cursor_updated)

func _on_drag_started(drag_data: DragDataInfo, position: Vector2, allowed_ops: int):
    browser_drag_data = drag_data
    
    if drag_data.is_link:
        # User is dragging a shop item link into the game
        var preview = _create_item_preview(drag_data.link_url)
        force_drag(drag_data, preview)

func _on_drag_cursor_updated(operation: int):
    # Update cursor based on drop target validity
    if operation == DragOperation.NONE:
        $DragPreview.modulate = Color.RED
    else:
        $DragPreview.modulate = Color.WHITE

func _create_item_preview(url: String) -> Control:
    var preview = TextureRect.new()
    preview.texture = preload("res://icons/item_placeholder.png")
    return preview

# In your inventory slot's _can_drop_data:
func _can_drop_data(at_position: Vector2, data) -> bool:
    if data is DragDataInfo and data.is_link:
        return _is_valid_shop_item(data.link_url)
    return false

func _drop_data(at_position: Vector2, data):
    if data is DragDataInfo and data.is_link:
        _add_item_from_url(data.link_url)
        cef_texture.drag_source_ended(at_position, DragOperation.COPY)

func _notification(what):
    if what == NOTIFICATION_DRAG_END:
        if cef_texture.is_dragging_from_browser():
            cef_texture.drag_source_system_ended()
        browser_drag_data = null
```
