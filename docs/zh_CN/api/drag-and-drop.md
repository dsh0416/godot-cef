# 拖放

CefTexture 支持 Godot 与嵌入式 CEF 浏览器之间的双向拖放（Drag & Drop）。这允许你：

- **从 Godot 拖动文件或数据到浏览器**（例如将文件拖放到网页上）
- **通过信号处理从浏览器发起的拖动**（例如从网页拖动图像或链接到您的游戏中）

## 概念

### 拖动操作

拖动操作由 `DragOperation` 类常量表示：

| 常量 | 值 | 描述 |
|------|-----|------|
| `NONE` | 0 | 不允许任何操作 |
| `COPY` | 1 | 复制拖动的数据 |
| `LINK` | 2 | 创建指向拖动数据的链接 |
| `MOVE` | 16 | 移动拖动的数据 |
| `EVERY` | MAX_INT | 允许所有操作 |

### DragDataInfo

当发生拖动事件时，您会收到一个包含正在拖动内容信息的 `DragDataInfo` 对象：

| 属性 | 类型 | 描述 |
|------|------|------|
| `is_link` | `bool` | 是否为 URL 链接拖动 |
| `is_file` | `bool` | 是否为文件拖动 |
| `is_fragment` | `bool` | 是否为文本/HTML 片段拖动 |
| `link_url` | `String` | 正在拖动的 URL（如果 `is_link`） |
| `link_title` | `String` | 链接的标题（如果 `is_link`） |
| `fragment_text` | `String` | 纯文本内容（如果 `is_fragment`） |
| `fragment_html` | `String` | HTML 内容（如果 `is_fragment`） |
| `file_names` | `Array[String]` | 文件路径列表（如果 `is_file`） |

## Godot → CEF 浏览器（将文件拖放到网页中）

要启用将文件或数据拖放到 CEF 浏览器中，您需要在处理 Godot 的拖放事件时调用 `CefTexture` 上的方法。

### 方法

#### `drag_enter(file_paths: Array[String], position: Vector2, allowed_ops: int)`

当拖动进入 `CefTexture` 区域时调用。这会通知 CEF 拖动操作正在开始。

```gdscript
func _can_drop_data(at_position: Vector2, data) -> bool:
    if data is Array:
        cef_texture.drag_enter(data, at_position, DragOperation.COPY)
        return true
    return false
```

#### `drag_over(position: Vector2, allowed_ops: int)`

当拖动在 `CefTexture` 上移动时重复调用。这会更新 CEF 的拖动位置。

```gdscript
func _process(delta):
    if is_dragging and cef_texture.is_drag_over():
        var mouse_pos = get_local_mouse_position()
        cef_texture.drag_over(mouse_pos, DragOperation.COPY)
```

#### `drag_leave()`

当拖动在未放下的情况下离开 `CefTexture` 区域时调用。

```gdscript
func _on_mouse_exited():
    if cef_texture.is_drag_over():
        cef_texture.drag_leave()
```

#### `drag_drop(position: Vector2)`

当用户释放拖动以将数据放到网页上时调用。

```gdscript
func _drop_data(at_position: Vector2, data):
    cef_texture.drag_drop(at_position)
```

### 完整示例：文件拖放区

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

## CEF 浏览器 → Godot（处理浏览器发起的拖动）

当用户开始从网页拖动内容（例如图像、链接或选中的文本）时，CefTexture 会发出您可以连接并在游戏中处理的信号。

### 信号

#### `drag_started(drag_data: DragDataInfo, position: Vector2, allowed_ops: int)`

当用户开始从网页拖动内容时发出。

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

当拖动光标视觉效果应根据当前位置允许的操作更改时发出。

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

当拖动操作从外部源进入 CefTexture 时发出（通过 CEF 拖动处理器）。

```gdscript
func _ready():
    cef_texture.drag_entered.connect(_on_drag_entered)

func _on_drag_entered(drag_data: DragDataInfo, mask: int):
    print("External drag entered with ops mask: ", mask)
```

### 通知 CEF 浏览器拖动结束

当从浏览器发起的拖动结束（被放下或取消）时，您应该通知 CEF：

#### `drag_source_ended(position: Vector2, operation: int)`

当浏览器发起的拖动以特定结果结束时调用。

```gdscript
func _on_drop_completed(drop_position: Vector2, was_accepted: bool):
    if cef_texture.is_dragging_from_browser():
        var op = DragOperation.COPY if was_accepted else DragOperation.NONE
        cef_texture.drag_source_ended(drop_position, op)
```

#### `drag_source_system_ended()`

当系统拖动操作结束时调用（清理）。

```gdscript
func _notification(what):
    if what == NOTIFICATION_DRAG_END:
        if cef_texture.is_dragging_from_browser():
            cef_texture.drag_source_system_ended()
```

### 查询方法

#### `is_dragging_from_browser() -> bool`

如果当前有从浏览器发起的活动拖动操作，返回 `true`。

#### `is_drag_over() -> bool`

如果当前有拖动操作在 CefTexture 上方，返回 `true`。

## 完整示例：处理浏览器拖动

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

