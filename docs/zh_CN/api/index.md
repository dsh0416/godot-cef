# API 参考

本节提供 `CefTexture` 节点的完整文档。`CefTexture` 可在 Godot 场景中将网页内容渲染为纹理（Texture）。

## 快速开始

安装 Godot CEF 插件后，您可以直接在场景中使用 `CefTexture`：

```gdscript
extends Control

func _ready():
    var cef_texture = CefTexture.new()
    cef_texture.url = "https://example.com"
    cef_texture.enable_accelerated_osr = true  # Enable GPU acceleration
    add_child(cef_texture)
```

## 概述

`CefTexture` 继承自 `TextureRect`，将基于 Chromium 的网页内容渲染为一张可交互的纹理。它支持：

- **GPU 加速渲染**：更高性能的离屏渲染（OSR）
- **完整 Web 能力**：JavaScript/HTML/CSS 全支持
- **双向通信（IPC）**：Godot ↔ JavaScript
- **输入处理**：鼠标、键盘与输入法（IME）
- **导航与状态**：前进/后退、加载状态等
- **拖放**：Godot 与网页间的双向拖放
- **下载处理**：拦截下载请求并由游戏侧决定策略

## 全局配置

由于 CEF 的架构限制，部分参数只能在 Godot 启动阶段设置**一次**。这些设置通过**项目设置**配置，并应用于所有 `CefTexture` 实例。

### 项目设置

在 Godot 中打开 **项目 > 项目设置 > godot_cef** 进行配置：

| 设置 | 描述 |
|------|------|
| `godot_cef/storage/data_path` | Cookie、缓存和 localStorage 的存储路径（默认：`user://cef-data`） |
| `godot_cef/storage/cache_size_mb` | 磁盘缓存最大容量（MB）（默认：`0` = CEF 默认） |
| `godot_cef/security/allow_insecure_content` | 允许在 HTTPS 页面中加载不安全（HTTP）内容 |
| `godot_cef/security/ignore_certificate_errors` | 忽略 SSL/TLS 证书错误 |
| `godot_cef/security/disable_web_security` | 禁用网页安全（CORS、同源策略） |
| `godot_cef/audio/enable_audio_capture` | 将浏览器音频通过 Godot 音频系统路由（默认：`false`） |
| `godot_cef/debug/remote_devtools_port` | Chrome DevTools 远程调试端口（默认：`9229`） |
| `godot_cef/performance/max_frame_rate` | 浏览器最大帧率（默认：`0` = 跟随 Godot FPS） |
| `godot_cef/network/user_agent` | 自定义 User-Agent 字符串（默认：空 = CEF 默认） |
| `godot_cef/network/proxy_server` | 代理服务器 URL（默认：空 = 直连） |
| `godot_cef/network/proxy_bypass_list` | 不走代理的主机列表（默认：空） |
| `godot_cef/advanced/custom_command_line_switches` | 自定义 CEF 命令行开关（每行一个） |

这些参数会在初始化期间以命令行开关的形式传递给 CEF 子进程，运行时无法修改。如需更改这些设置，请重启 Godot 应用程序。

::: warning
安全相关选项风险较高，只应在明确的场景下启用。如果启用了任何安全选项，启动时会打印警告日志。
:::

## 远程 DevTools（开发者工具）

远程 DevTools 允许您使用 Chrome DevTools 调试运行在 Godot 中的网页内容：查看 DOM、调试 JavaScript、监控网络请求、做性能分析等。

### 可用性

出于安全考虑，远程调试**仅在以下情况下启用**：
- Godot 在**调试模式**下运行（`OS.is_debug_build()` 返回 `true`），或
- 从 **Godot 编辑器**运行（`Engine.is_editor_hint()` 返回 `true`）

远程调试在生产/发布版本中会自动禁用。

### 访问开发者工具

启用远程调试后，CEF 会监听配置的端口（默认：**9229**）。您可以通过 `godot_cef/debug/remote_devtools_port` 项目设置修改端口。

1. 打开 Chrome 并导航至 `chrome://inspect`
2. 点击 “发现网络目标（Discover network targets）” 旁边的 **“配置…”**
3. 将 `localhost:<端口>` 添加到目标发现列表（例如 `localhost:9229`）
4. 您的 CEF 浏览器实例将出现在 “Remote Target/远程目标” 下
5. 点击 **"inspect"** 打开该页面的开发者工具

### 常见用法

- **调试 JavaScript 错误**（Web UI）
- **实时检查与修改 DOM**
- **监控网络请求**（调试 API 调用）
- **性能分析**（定位瓶颈）
- **临时验证 CSS 改动**（再决定是否固化到项目中）

## API 章节

- [**属性**](./properties.md) - 节点属性和配置
- [**方法**](./methods.md) - 控制浏览器的可用方法
- [**信号**](./signals.md) - CefTexture 节点发出的事件
- [**音频捕获**](./audio-capture.md) - 将浏览器音频接入 Godot 音频系统
- [**输入法（IME）支持**](./ime-support.md) - 输入法（IME）集成
- [**拖放**](./drag-and-drop.md) - 双向拖放支持
- [**下载**](./downloads.md) - 处理网页文件下载

## 基本使用示例

```gdscript
extends Node2D

@onready var browser = $CefTexture

func _ready():
    # Set initial URL
    browser.url = "https://example.com"

    # Connect to signals
    browser.load_finished.connect(_on_page_loaded)
    browser.ipc_message.connect(_on_message_received)

func _on_page_loaded(url: String, status: int):
    print("Page loaded: ", url)

    # Execute JavaScript
    browser.eval("document.body.style.backgroundColor = '#f0f0f0'")

func _on_message_received(message: String):
    print("Received from web: ", message)
```

## 导航

```gdscript
# Navigate to URLs
browser.url = "https://godotengine.org"

# Browser controls
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
    print("Download: %s (%s)" % [info.suggested_file_name, info.mime_type])

func _on_download_updated(info: DownloadUpdateInfo):
    if info.is_complete:
        print("Download complete: ", info.full_path)
    elif info.is_in_progress:
        print("Progress: %d%%" % info.percent_complete)
```

