# 显示事件与可见性

`CefTexture` 和 `CefTexture2D` 提供相同的五个显示信号：

| 信号 | 值 |
|------|----|
| `tooltip_changed(text: String)` | tooltip 文本，空字符串用于清除提示。 |
| `fullscreen_changed(fullscreen: bool)` | 网页内容是否处于全屏模式。 |
| `loading_progress_changed(progress: float)` | CEF 估计的加载进度，范围为 `0.0`–`1.0`。 |
| `status_message_changed(message: String)` | 浏览器状态文本，包括悬停链接的目标 URL，空字符串用于清除消息。 |
| `favicon_urls_changed(urls: PackedStringArray)` | 候选 favicon URL，空数组用于清除候选项。 |

将这些信号连接到 Godot 界面。集成层不会自动创建原生 tooltip，也不会自动
将 Godot 窗口切换为全屏。favicon 事件提供 URL，不会下载图像。加载进度是
估计值，不代表字节或资源数量；需要加载完成或错误结果时，使用 `CefTexture`
对应的加载信号。

## Godot 界面示例

此示例使用名为 `Browser` 的 `CefTexture` 子节点、一个 `ProgressBar`，以及
`StatusLabel` 和 `FaviconURLLabel` 两个标签。全屏处理函数展示一种应用策略，
退出全屏时恢复先前的窗口模式。

```gdscript
extends Control

@onready var browser: CefTexture = $Browser
var favicon_urls := PackedStringArray()
var _browser_fullscreen := false
var _previous_window_mode := Window.MODE_WINDOWED

func _ready() -> void:
    browser.tooltip_changed.connect(_on_tooltip_changed)
    browser.fullscreen_changed.connect(_on_fullscreen_changed)
    browser.loading_progress_changed.connect(_on_loading_progress_changed)
    browser.status_message_changed.connect(_on_status_message_changed)
    browser.favicon_urls_changed.connect(_on_favicon_urls_changed)
    $ProgressBar.max_value = 100.0

func _on_tooltip_changed(text: String) -> void:
    browser.tooltip_text = text

func _on_loading_progress_changed(progress: float) -> void:
    $ProgressBar.value = progress * 100.0
    $ProgressBar.visible = progress < 1.0

func _on_status_message_changed(message: String) -> void:
    $StatusLabel.text = message

func _on_favicon_urls_changed(urls: PackedStringArray) -> void:
    favicon_urls = urls
    $FaviconURLLabel.text = "" if urls.is_empty() else urls[0]
    # 应用可以选择 URL，自行下载并解码支持的图像格式。

func _on_fullscreen_changed(fullscreen: bool) -> void:
    if fullscreen == _browser_fullscreen:
        return
    _browser_fullscreen = fullscreen
    if fullscreen:
        _previous_window_mode = get_window().mode
        get_window().mode = Window.MODE_FULLSCREEN
    else:
        get_window().mode = _previous_window_mode
```

这些信号同样适用于 `CefTexture2D` 引用，此时应将 tooltip 文本应用到显示
该纹理的 Godot `Control`。`load_finished`、`title_changed`、`console_message`
等其他节点事件仍仅适用于 `CefTexture`。

## 隐藏和恢复渲染

两种浏览器都有默认值为 `false` 的 `browser_hidden` 属性。设置它会通知
CEF 离屏浏览器已隐藏，并停止请求外部 begin frame。最新纹理仍可使用；
清除此属性即可恢复渲染，无需重建浏览器或重新加载页面。

```gdscript
browser.browser_hidden = true
print(browser.get_browser_hidden())  # 手动设置：true。
print(browser.is_browser_hidden())   # 实际渲染隐藏状态：true。
browser.set_browser_hidden(false)    # 恢复；自动隐藏策略仍可能生效。
```

`CefTexture` 还提供默认值为 `false` 的 `pause_rendering_when_hidden` 属性。
开启后，`is_visible_in_tree()` 为 false 时会隐藏浏览器渲染，包括祖先节点
被隐藏的情况。实际隐藏状态为手动设置或自动隐藏状态任一成立：

```gdscript
browser.pause_rendering_when_hidden = true
$BrowserPanel.hide()  # 隐藏祖先节点也会隐藏浏览器渲染。
$BrowserPanel.show()  # browser_hidden 为 false 时恢复。
```

`get_browser_hidden()` 仅返回手动设置，`is_browser_hidden()` 还包含自动策略。
手动设置 `browser_hidden = true` 后，即使节点可见，浏览器渲染也会保持隐藏。

`CefTexture2D` 没有自动节点可见性策略，因为同一个资源可能被多个精灵或
材质共享。使用方应在所有视图都不需要渲染时显式设置 `browser_hidden`。

隐藏渲染期间仍保留 CEF 任务调度、JavaScript、网络活动、音频、显示信号和
浏览器生命周期处理。它不会暂停应用，也不会撤销权限。场景处理设置及
主循环降频仍然适用，需要时应单独控制音频和权限。
