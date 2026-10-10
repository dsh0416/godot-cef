# File and JavaScript dialogs

`CefTexture` and `CefTexture2D` expose the same controls for CEF file choosers,
JavaScript `alert()`, `confirm()` and `prompt()`, and before-unload confirmation.
Your application can answer these requests asynchronously with Godot UI.
The two examples below are independent; combine their signal connections if
your application needs both file choosers and JavaScript dialogs.

## Policy and lifetime

Set `dialog_policy` on the browser:

| Value | Name | Behavior |
|-------|------|----------|
| `0` (default) | Native | Leave dialogs to CEF's default implementation |
| `1` | Signal | Emit a request signal for the application to answer |
| `2` | Cancel | Cancel requests without showing a dialog |

Native preserves the existing CEF behavior. Its dialogs depend on the platform
and may use operating-system UI, which can be unsuitable for a headless run.
Signal does not create Godot UI automatically: connect the request signals and
provide your own `FileDialog`, `AcceptDialog`, `ConfirmationDialog`, or other UI.
Connect before enabling Signal or loading a page that can request a dialog.
A Signal request with no listener for its request signal is canceled.

`godot_cef/dialogs/request_timeout_seconds` sets the response deadline (default
`60` seconds, minimum `1`). Configure it before browser creation. An unanswered
request is canceled at its deadline. Timeout processing follows the Godot main
loop; a blocked main thread can delay notifications. Answer methods also check
the deadline and reject late answers.

Each request's CEF callback is completed at most once. Answer methods return
`false` for unknown, stale, expired, or completed IDs, or an ID belonging to the
other dialog type. `true` means the answer was accepted, not that a file operation
or navigation has already succeeded. An empty file selection cancels the chooser.
An invalid path selection returns `false` and leaves the request pending so the
application can correct it or cancel.
Changing `dialog_policy`, main-frame navigation, renderer termination, and browser closure
cancel pending requests. CEF's JavaScript dialog reset/closed callbacks cancel
pending JavaScript dialogs. IDs belong to a browser instance;
do not save or reuse them across instances.

Use `dialog_request_finished` to remove a pending request and close its UI.
Finished signals are delivered while the Godot object remains alive. Freeing
the node or resource does not guarantee a final signal; hide dialogs and clear
their IDs in your own browser teardown path too. This policy is separate from
[`permission_policy`](./permissions.md) and [`popup_policy`](./properties.md#popup-policy).

| Finished result | Meaning |
|-----------------|---------|
| `accepted` | Nonempty file selection or a JavaScript answer with `accepted = true` |
| `canceled` | Explicit cancellation, an empty file selection, or `accepted = false` |
| `timed_out` | The response deadline expired |
| `unhandled` | Signal mode had no listener for this request type |
| `navigation` | Main-frame navigation invalidated the request |
| `policy_changed` | The browser's dialog policy changed |
| `reset` | CEF reset its JavaScript dialog state |
| `dialog_closed` | CEF reported that its JavaScript dialog closed |
| `browser_closed` | The browser closed |
| `renderer_terminated` | The browser's render process terminated |

## Signals and methods

These APIs are available on both browser types. GDScript `int` holds the signed
64-bit `request_id`.

| API | Meaning |
|-----|---------|
| `file_dialog_requested(info: FileDialogRequestInfo)` | Ask the application to select files, a folder, or a save path |
| `js_dialog_requested(info: JsDialogRequestInfo)` | Ask the application to answer a JavaScript or before-unload dialog |
| `dialog_request_finished(request_id: int, result: String)` | Remove the application's pending UI for this ID |
| `continue_file_dialog(request_id: int, file_paths: PackedStringArray) -> bool` | Return the selected absolute filesystem paths; an empty array cancels |
| `cancel_file_dialog(request_id: int) -> bool` | Cancel a pending file chooser |
| `continue_js_dialog(request_id: int, accepted: bool, user_input: String) -> bool` | Answer the dialog; `user_input` is used by `prompt()` |
| `cancel_js_dialog(request_id: int) -> bool` | Cancel/reject a pending JavaScript dialog |
| `is_dialog_pending(request_id: int) -> bool` | Whether this ID still accepts an answer |

For `confirm()`, `accepted` is the JavaScript boolean result. For `prompt()`,
`true` returns `user_input` and `false` cancels the prompt. For before-unload,
`true` allows leaving/reloading and `false` keeps the current page. Acknowledge
an `alert()` with `accepted = true` and an empty `user_input`.

## FileDialogRequestInfo

| Property | Type | Meaning |
|----------|------|---------|
| `request_id` | `int` | ID used when answering this request |
| `mode` | `int` | CEF mode: `0` open, `1` open multiple, `2` open folder, `3` save |
| `title` | `String` | Requested title; may be empty |
| `default_file_path` | `String` | Suggested filesystem path or filename; may be empty |
| `accept_filters` | `PackedStringArray` | CEF accept values such as `.png` or `image/*` |
| `accept_extensions` | `PackedStringArray` | Corresponding extension groups, separated by semicolons, e.g. `.png;.jpg`; a group may be empty |
| `accept_descriptions` | `PackedStringArray` | Corresponding human-readable descriptions; may be empty |

The three accept arrays describe the same filter entries. CEF filters are not
Godot filter strings: translate the extension groups into Godot glob patterns,
and choose a fallback for MIME types without known extensions. Filters help the
user select files; they are not a validation boundary.

Use `FileDialog.ACCESS_FILESYSTEM` so selection signals provide operating-system
paths. Pass absolute paths to `continue_file_dialog()`, not URLs or unresolved
`res://`/`user://` paths. For an application-provided Godot path, resolve it first
with `ProjectSettings.globalize_path()`. Exported `res://` files inside a PCK are
not necessarily individual filesystem files available to CEF.
Only open-multiple mode accepts more than one path; other modes require exactly
one path when accepting the request.

### Connecting a Godot FileDialog

Add `CefTexture` and `FileDialog` children to this `Control`. This compact example
handles one file chooser at a time and cancels an additional request while its
UI is occupied. Replace `browser` with a `CefTexture2D` reference to use it with
a resource-backed browser.

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
    chooser.title = info.title if not info.title.is_empty() else "Select a path"
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

This example leaves filters unrestricted when CEF cannot expand any extension
group. Apply your application's file-type and path rules before returning paths.
File selection is separate from the [file-download interface](./downloads.md).

## JsDialogRequestInfo

| Property | Type | Meaning |
|----------|------|---------|
| `request_id` | `int` | ID used when answering this request |
| `kind` | `String` | `alert`, `confirm`, `prompt`, or `before_unload` |
| `origin_url` | `String` | URL reported by CEF for the JavaScript dialog; may be empty for before-unload |
| `message_text` | `String` | Message reported by CEF |
| `default_prompt_text` | `String` | Initial prompt input; empty for other dialog kinds |
| `is_reload` | `bool` | Whether a before-unload request concerns a reload |

Page-provided text and URLs are display data. Keep your application's own title
and confirm/cancel labels so a page cannot choose the meaning of its buttons.

### Connecting Godot JavaScript dialogs

Add these children to a `Control`: `CefTexture`, `Alert` (`AcceptDialog`),
`Confirm` (`ConfirmationDialog`), and `Prompt` (`ConfirmationDialog`) containing
a `LineEdit` named `Input`. The same confirmation UI handles `confirm()` and
before-unload, with explicit Leave/Stay labels for the latter.

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
            alert.title = "Page message"
            alert.dialog_text = info.message_text
            alert.popup_centered()
        "confirm", "before_unload":
            confirm.title = "Page confirmation"
            confirm.dialog_text = info.message_text
            confirm.ok_button_text = "OK"
            confirm.cancel_button_text = "Cancel"
            if info.kind == "before_unload":
                confirm.dialog_text = "Reload this page?" if info.is_reload else "Leave this page?"
                confirm.ok_button_text = "Reload" if info.is_reload else "Leave"
                confirm.cancel_button_text = "Stay"
            confirm.popup_centered()
        "prompt":
            prompt.title = "Page input"
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

Keep the UI under the same owner as its browser. If you free the browser
separately, clear the active ID and hide the UI in that teardown path. For a
headless application, use Cancel or answer Signal requests programmatically.
