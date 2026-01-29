# API 参考

本节提供 `CefTexture` 节点的完整文档，该节点允许您在 Godot 场景中将网页内容渲染为纹理。

## 快速开始

安装 Godot CEF 插件后，您可以在场景中使用 `CefTexture` 节点：

```gdscript
extends Control

func _ready():
    var cef_texture = CefTexture.new()
    cef_texture.url = "https://example.com"
    cef_texture.enable_accelerated_osr = true  # 启用 GPU 加速
    add_child(cef_texture)
```

## 概述

`CefTexture` 节点继承自 `TextureRect`，提供基于 Chromium 的网页浏览器作为纹理渲染。它支持：

- **GPU 加速渲染** 实现高性能
- **交互式网页内容** 完整支持 JavaScript
- **双向通信** Godot 与 JavaScript 之间
- **输入处理** 包括鼠标、键盘和输入法支持
- **导航控制** 和浏览器状态管理
- **拖放** Godot 与网页内容之间
- **下载处理** 完全控制文件下载

## 全局配置

由于 CEF 的架构限制，某些参数只能在 Godot 启动过程中配置**一次**。这些设置通过**项目设置**配置，并应用于所有 `CefTexture` 实例。

### 项目设置

导航至 **项目 > 项目设置 > godot_cef** 进行配置：

| 设置 | 描述 |
|------|------|
| `godot_cef/storage/data_path` | Cookie、缓存和 localStorage 的存储路径（默认：`user://cef-data`） |
| `godot_cef/security/allow_insecure_content` | 允许在 HTTPS 页面中加载不安全（HTTP）内容 |
| `godot_cef/security/ignore_certificate_errors` | 忽略 SSL/TLS 证书错误 |
| `godot_cef/security/disable_web_security` | 禁用网页安全（CORS、同源策略） |
| `godot_cef/audio/enable_audio_capture` | 将浏览器音频通过 Godot 音频系统路由（默认：`false`） |
| `godot_cef/debug/remote_devtools_port` | Chrome DevTools 远程调试端口（默认：`9229`） |

这些参数在初始化期间作为命令行开关传递给 CEF 子进程，运行时无法修改。如需更改这些设置，必须重启 Godot 应用程序。

::: warning
安全设置是危险的，只应在特定用例下启用。如果启用了任何安全设置，启动时会记录警告。
:::

## 远程开发者工具

远程开发者工具允许您使用 Chrome 开发者工具调试 Godot 应用程序中运行的网页内容。这对于检查 DOM、调试 JavaScript、监控网络请求和性能分析非常有用。

### 可用性

出于安全考虑，远程调试**仅在以下情况下启用**：
- Godot 在**调试模式**下运行（`OS.is_debug_build()` 返回 `true`），或
- 从 **Godot 编辑器**运行（`Engine.is_editor_hint()` 返回 `true`）

远程调试在生产/发布版本中会自动禁用。

### 访问开发者工具

启用远程调试后，CEF 会监听配置的端口（默认：**9229**）。您可以通过 `godot_cef/debug/remote_devtools_port` 项目设置更改此端口。

1. 打开 Chrome 并导航至 `chrome://inspect`
2. 点击"发现网络目标"旁边的 **"配置..."**
3. 将 `localhost:<端口>` 添加到目标发现列表（例如 `localhost:9229`）
4. 您的 CEF 浏览器实例将出现在"远程目标"下
5. 点击 **"inspect"** 打开该页面的开发者工具

### 用例

- **调试 JavaScript 错误** 在您的 Web UI 中
- **实时检查和修改 DOM** 元素
- **监控网络请求** 调试 API 调用
- **性能分析** 识别瓶颈
- **测试 CSS 更改** 在永久应用之前

## API 章节

- [**属性**](./properties.md) - 节点属性和配置
- [**方法**](./methods.md) - 控制浏览器的可用方法
- [**信号**](./signals.md) - CefTexture 节点发出的事件
- [**音频捕获**](./audio-capture.md) - 将浏览器音频通过 Godot 音频系统路由
- [**输入法支持**](./ime-support.md) - 输入法编辑器集成
- [**拖放**](./drag-and-drop.md) - 双向拖放支持
- [**下载**](./downloads.md) - 处理网页文件下载

## 基本使用示例

```gdscript
extends Node2D

@onready var browser = $CefTexture

func _ready():
    # 设置初始 URL
    browser.url = "https://example.com"

    # 连接信号
    browser.load_finished.connect(_on_page_loaded)
    browser.ipc_message.connect(_on_message_received)

func _on_page_loaded(url: String, status: int):
    print("页面已加载: ", url)

    # 执行 JavaScript
    browser.eval("document.body.style.backgroundColor = '#f0f0f0'")

func _on_message_received(message: String):
    print("从网页收到: ", message)
```

## 导航

```gdscript
# 导航到 URL
browser.url = "https://godotengine.org"

# 浏览器控制
if browser.can_go_back():
    browser.go_back()

if browser.can_go_forward():
    browser.go_forward()

browser.reload()
browser.reload_ignore_cache()
```

## 下载处理

```gdscript
func _ready():
    browser.download_requested.connect(_on_download_requested)
    browser.download_updated.connect(_on_download_updated)

func _on_download_requested(info: DownloadRequestInfo):
    print("下载: %s (%s)" % [info.suggested_file_name, info.mime_type])

func _on_download_updated(info: DownloadUpdateInfo):
    if info.is_complete:
        print("下载完成: ", info.full_path)
    elif info.is_in_progress:
        print("进度: %d%%" % info.percent_complete)
```

