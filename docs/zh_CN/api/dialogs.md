# 文件与 JavaScript 对话框

`CefTexture` 和 `CefTexture2D` 提供相同的 CEF 文件选择、JavaScript `alert()`、
`confirm()`、`prompt()` 与离开页面确认接口。应用可以使用 Godot 界面异步回答请求。
下方两个示例独立使用；应用同时需要文件选择与 JavaScript 对话框时，应合并它们的
信号连接。

## 策略与生命周期

通过浏览器的 `dialog_policy` 选择行为：

| 值 | 名称 | 行为 |
|----|------|------|
| `0`（默认） | Native | 保留 CEF 的默认对话框实现 |
| `1` | Signal | 发出请求信号，由应用回答 |
| `2` | Cancel | 直接取消请求，不显示对话框 |

Native 保留现有 CEF 行为，对话框取决于平台，可能使用操作系统界面，
不一定适合无界面运行。Signal 不会自动创建 Godot 界面：应用需连接请求信号，
自行提供 `FileDialog`、`AcceptDialog`、`ConfirmationDialog` 或其他界面。
请在启用 Signal 或加载可能触发对话框的页面前连接信号。
Signal 请求没有对应的信号监听器时会被取消。

`godot_cef/dialogs/request_timeout_seconds` 设置应答期限，默认 `60` 秒，最小 `1`。
请在创建浏览器之前配置。未回答的请求到期后取消。超时处理遵循 Godot 主循环，
阻塞主线程可能延迟通知；应答方法也会检查实际期限并拒绝过期回答。

每个请求的 CEF 回调最多完成一次。对不存在、已失效、已超时或已完成的 ID，
或对另一种对话框的 ID 调用应答方法会返回 `false`。`true` 表示回答已被接受，
不代表文件操作或导航已经成功。空文件列表表示取消选择。
无效的路径列表会返回 `false`，并保留待处理请求，应用可以修正路径或取消请求。
更改 `dialog_policy`、主框架导航、渲染进程终止与浏览器关闭都会取消待处理请求。
CEF 的 JavaScript 对话框重置 / 关闭回调会取消待处理的 JavaScript 对话框。
ID 属于特定浏览器实例，不要保存或跨实例复用。

通过 `dialog_request_finished` 移除待处理请求并关闭其界面。
只有 Godot 对象仍存活时才能送达完成信号。释放节点或资源时不保证发出最终信号，
因此在应用自己的浏览器清理流程中也应隐藏界面并清空 ID。
此策略独立于 [`permission_policy`](./permissions.md) 和
[`popup_policy`](./properties.md#弹出策略)。

| 完成结果 | 含义 |
|----------|------|
| `accepted` | 非空文件选择，或 `accepted = true` 的 JavaScript 回答 |
| `canceled` | 显式取消、空文件列表，或 `accepted = false` |
| `timed_out` | 应答期限已过 |
| `unhandled` | Signal 模式下没有此类请求的监听器 |
| `navigation` | 主框架导航使请求失效 |
| `policy_changed` | 浏览器的对话框策略更改 |
| `reset` | CEF 重置 JavaScript 对话框状态 |
| `dialog_closed` | CEF 报告 JavaScript 对话框已关闭 |
| `browser_closed` | 浏览器已关闭 |
| `renderer_terminated` | 浏览器的渲染进程已终止 |

## 信号与方法

下列接口在两种浏览器类型上均可用。GDScript 的 `int` 可保存有符号 64 位
`request_id`。

| API | 含义 |
|-----|------|
| `file_dialog_requested(info: FileDialogRequestInfo)` | 请求应用选择文件、文件夹或保存路径 |
| `js_dialog_requested(info: JsDialogRequestInfo)` | 请求应用回答 JavaScript 对话框或离开页面确认 |
| `dialog_request_finished(request_id: int, result: String)` | 清理此 ID 对应的待处理界面 |
| `continue_file_dialog(request_id: int, file_paths: PackedStringArray) -> bool` | 返回选中的绝对文件系统路径；空数组表示取消 |
| `cancel_file_dialog(request_id: int) -> bool` | 取消待处理的文件选择 |
| `continue_js_dialog(request_id: int, accepted: bool, user_input: String) -> bool` | 回答对话框；`user_input` 用于 `prompt()` |
| `cancel_js_dialog(request_id: int) -> bool` | 取消或拒绝待处理的 JavaScript 对话框 |
| `is_dialog_pending(request_id: int) -> bool` | 此 ID 是否仍可回答 |

对于 `confirm()`，`accepted` 是 JavaScript 收到的布尔结果。对于 `prompt()`，
`true` 返回 `user_input`，`false` 表示取消。离开页面确认中，`true` 允许离开或重载，
`false` 保留当前页面。确认 `alert()` 时传入 `accepted = true` 和空的 `user_input`。

## FileDialogRequestInfo

| 属性 | 类型 | 含义 |
|------|------|------|
| `request_id` | `int` | 回答此请求时使用的 ID |
| `mode` | `int` | CEF 模式：`0` 打开，`1` 打开多个文件，`2` 打开文件夹，`3` 保存 |
| `title` | `String` | 请求的标题，可能为空 |
| `default_file_path` | `String` | 建议的文件系统路径或文件名，可能为空 |
| `accept_filters` | `PackedStringArray` | CEF accept 值，例如 `.png` 或 `image/*` |
| `accept_extensions` | `PackedStringArray` | 对应的扩展名组，以分号分隔，例如 `.png;.jpg`；组可能为空 |
| `accept_descriptions` | `PackedStringArray` | 对应的可读描述，可能为空 |

三个 accept 数组描述相同的筛选项。CEF 筛选值不是 Godot 筛选字符串，
应将扩展名组转换为 Godot 的 glob 模式；无法解析出扩展名的 MIME 类型需要应用
选择回退方式。筛选项帮助用户选择文件，不构成验证边界。

使用 `FileDialog.ACCESS_FILESYSTEM`，使选择信号返回操作系统路径。
传入 `continue_file_dialog()` 的应是绝对路径，不是 URL 或尚未解析的
`res://`、`user://` 路径。应用提供的 Godot 路径需先用
`ProjectSettings.globalize_path()` 转换。导出后 PCK 内的 `res://` 资源不一定是
CEF 可以直接访问的独立文件。
只有打开多个文件模式接受多个路径；其他模式接受请求时必须恰好提供一个路径。

### 连接 Godot FileDialog

为下面的 `Control` 添加 `CefTexture` 和 `FileDialog` 子节点。此简短示例每次
处理一个文件选择；界面被占用时会取消新请求。将 `browser` 替换为
`CefTexture2D` 引用，也可用于资源浏览器。

```gdscript
extends Control

@onready var browser: CefTexture = $CefTexture
@onready var chooser: FileDialog = $FileDialog
var active_file_id: int = -1

func _ready() -> void:
    chooser.access = FileDialog.ACCESS_FILESYSTEM
    chooser.file_selected.connect(_on_file_selected)
    chooser.files_selected.connect(_on_files_selected)
    chooser.dir_selected.connect(_on_file_selected)
    chooser.canceled.connect(_cancel_file)
    browser.file_dialog_requested.connect(_on_file_requested)
    browser.dialog_request_finished.connect(_on_dialog_finished)
    browser.dialog_policy = 1

func _on_file_requested(info: FileDialogRequestInfo) -> void:
    if not browser.is_dialog_pending(info.request_id):
        return
    if active_file_id != -1:
        browser.cancel_file_dialog(info.request_id)
        return
    var modes := [FileDialog.FILE_MODE_OPEN_FILE, FileDialog.FILE_MODE_OPEN_FILES,
        FileDialog.FILE_MODE_OPEN_DIR, FileDialog.FILE_MODE_SAVE_FILE]
    if info.mode < 0 or info.mode >= modes.size():
        browser.cancel_file_dialog(info.request_id)
        return
    active_file_id = info.request_id
    chooser.file_mode = modes[info.mode]
    chooser.title = info.title if not info.title.is_empty() else "选择路径"
    chooser.clear_filters()
    for index in range(info.accept_extensions.size()):
        var patterns := PackedStringArray()
        for extension in info.accept_extensions[index].split(";", false):
            patterns.append("*" + extension)
        if not patterns.is_empty():
            chooser.add_filter(",".join(patterns), info.accept_descriptions[index])
    if not info.default_file_path.is_empty():
        chooser.current_path = info.default_file_path
    chooser.popup_centered_ratio(0.7)

func _on_file_selected(path: String) -> void:
    _on_files_selected(PackedStringArray([path]))

func _on_files_selected(paths: PackedStringArray) -> void:
    var id := active_file_id
    active_file_id = -1
    chooser.hide()
    if is_instance_valid(browser):
        if not browser.continue_file_dialog(id, paths) and browser.is_dialog_pending(id):
            browser.cancel_file_dialog(id)

func _cancel_file() -> void:
    var id := active_file_id
    active_file_id = -1
    chooser.hide()
    if is_instance_valid(browser):
        browser.cancel_file_dialog(id)

func _on_dialog_finished(id: int, _result: String) -> void:
    if active_file_id == id:
        active_file_id = -1
        chooser.hide()
```

CEF 无法展开任何扩展名组时，此示例不限制筛选项。返回路径前应执行应用所需的
文件类型与路径验证。文件选择与[文件下载接口](./downloads.md)相互独立。

## JsDialogRequestInfo

| 属性 | 类型 | 含义 |
|------|------|------|
| `request_id` | `int` | 回答此请求时使用的 ID |
| `kind` | `String` | `alert`、`confirm`、`prompt` 或 `before_unload` |
| `origin_url` | `String` | CEF 提供的 JavaScript 对话框来源 URL；离开页面确认中可能为空 |
| `message_text` | `String` | CEF 提供的消息 |
| `default_prompt_text` | `String` | 输入框初始文本，其他类型为空 |
| `is_reload` | `bool` | 离开页面请求是否由重载引起 |

页面提供的文本与 URL 是显示数据。请使用应用自己的标题及确认、取消按钮文字，
避免页面自行定义按钮含义。

### 连接 Godot JavaScript 对话框

为 `Control` 添加这些子节点：`CefTexture`、`Alert`（`AcceptDialog`）、
`Confirm`（`ConfirmationDialog`）和 `Prompt`（`ConfirmationDialog`）。
在 `Prompt` 内放一个名为 `Input` 的 `LineEdit`。同一个确认界面处理 `confirm()`
和离开页面确认，后者明确使用“离开 / 留在页面”按钮。

```gdscript
extends Control

@onready var browser: CefTexture = $CefTexture
@onready var alert: AcceptDialog = $Alert
@onready var confirm: ConfirmationDialog = $Confirm
@onready var prompt: ConfirmationDialog = $Prompt
@onready var prompt_input: LineEdit = $Prompt/Input
var active_js_id: int = -1
var active_kind: String = ""

func _ready() -> void:
    alert.confirmed.connect(_answer_js.bind(true))
    alert.canceled.connect(_answer_js.bind(false))
    confirm.confirmed.connect(_answer_js.bind(true))
    confirm.canceled.connect(_answer_js.bind(false))
    prompt.confirmed.connect(_answer_js.bind(true))
    prompt.canceled.connect(_answer_js.bind(false))
    browser.js_dialog_requested.connect(_on_js_requested)
    browser.dialog_request_finished.connect(_on_dialog_finished)
    browser.dialog_policy = 1

func _on_js_requested(info: JsDialogRequestInfo) -> void:
    if not browser.is_dialog_pending(info.request_id):
        return
    if active_js_id != -1:
        browser.cancel_js_dialog(info.request_id)
        return
    active_js_id = info.request_id
    active_kind = info.kind
    match info.kind:
        "alert":
            alert.title = "页面消息"
            alert.dialog_text = info.message_text
            alert.popup_centered()
        "confirm", "before_unload":
            confirm.title = "页面确认"
            confirm.dialog_text = info.message_text
            confirm.ok_button_text = "确定"
            confirm.cancel_button_text = "取消"
            if info.kind == "before_unload":
                confirm.dialog_text = "是否重载页面？" if info.is_reload else "是否离开页面？"
                confirm.ok_button_text = "重载" if info.is_reload else "离开"
                confirm.cancel_button_text = "留在页面"
            confirm.popup_centered()
        "prompt":
            prompt.title = "页面输入"
            prompt.dialog_text = info.message_text
            prompt_input.text = info.default_prompt_text
            prompt.popup_centered()
            prompt_input.grab_focus()
        _:
            _answer_js(false)

func _answer_js(accepted: bool) -> void:
    var id := active_js_id
    var input := prompt_input.text if active_kind == "prompt" else ""
    _clear_js_ui()
    if is_instance_valid(browser):
        browser.continue_js_dialog(id, accepted, input)

func _clear_js_ui() -> void:
    active_js_id = -1
    active_kind = ""
    alert.hide()
    confirm.hide()
    prompt.hide()

func _on_dialog_finished(id: int, _result: String) -> void:
    if active_js_id == id:
        _clear_js_ui()
```

建议让界面与浏览器具有相同的所有者。应用单独释放浏览器时，也应在相应流程中
清空活动 ID 并隐藏界面。无界面应用可使用 Cancel，或通过代码回答 Signal 请求。
