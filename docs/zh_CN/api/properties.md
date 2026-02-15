# 属性

`CefTexture` 提供多项属性，用于配置与状态管理。

`CefTexture2D` 是一个仅渲染的 `Texture2D` 资源变体，与 `CefTexture`
共享同一套浏览器/渲染后端。它可以直接赋值给 `Sprite2D.texture`
以及 3D 材质纹理槽位。

## 节点属性

| 属性 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `url` | `String` | `"https://google.com"` | 要显示的 URL。设置该属性会让浏览器导航到新地址；读取时返回当前 URL（可能因用户操作/重定向而变化）。 |
| `enable_accelerated_osr` | `bool` | `true` | 启用 GPU 加速渲染 |
| `background_color` | `Color` | `Color(0, 0, 0, 0)` | 浏览器背景色。将 alpha 设为 0 表示透明背景，或使用实色以禁用透明效果。 |
| `popup_policy` | `int` | `0` | 控制弹出窗口的处理方式。`0` = BLOCK（静默阻止），`1` = REDIRECT（在当前浏览器中导航到弹出 URL），`2` = SIGNAL_ONLY（触发 `popup_requested` 信号）。可在运行时更改。 |

## CefTexture2D 属性

| 属性 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `url` | `String` | `"https://google.com"` | 该资源实例加载的 URL。 |
| `enable_accelerated_osr` | `bool` | `true` | 在支持的平台启用加速 OSR，否则自动回退到软件渲染。 |
| `background_color` | `Color` | `Color(0, 0, 0, 0)` | 浏览器背景色（支持透明）。 |
| `popup_policy` | `int` | `0` | 弹窗策略：BLOCK / REDIRECT / SIGNAL_ONLY。 |
| `texture_size` | `Vector2i` | `Vector2i(1024, 1024)` | 浏览器纹理逻辑尺寸（像素）。 |

`CefTexture2D` 的 v1 版本刻意保持为仅渲染：不包含内置的 3D 表面输入映射/射线投射路由。

```gdscript
var browser_tex := CefTexture2D.new()
browser_tex.url = "https://example.com"
browser_tex.texture_size = Vector2i(1024, 1024)
$Sprite2D.texture = browser_tex
```

```gdscript
var browser_tex := CefTexture2D.new()
browser_tex.url = "https://example.com"
var mat := StandardMaterial3D.new()
mat.albedo_texture = browser_tex
$MeshInstance3D.set_surface_override_material(0, mat)
```

## 项目设置

应用于**所有** `CefTexture` 和 `CefTexture2D` 实例的全局设置在 **项目设置 > godot_cef** 中配置。这些设置必须在任何 `CefTexture` 或 `CefTexture2D` 进入场景树之前设置。

### 存储设置

| 设置 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `godot_cef/storage/data_path` | `String` | `"user://cef-data"` | Cookie、缓存和 localStorage 的存储路径。支持 `user://` 和 `res://` 协议。 |

### 安全设置

::: danger 安全警告
这些设置存在较高安全风险，只应在明确的场景下启用（例如加载本地开发内容）。在生产环境启用可能会使用户面临安全漏洞。
:::

| 设置 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `godot_cef/security/allow_insecure_content` | `bool` | `false` | 允许在 HTTPS 页面中加载 HTTP 内容 |
| `godot_cef/security/ignore_certificate_errors` | `bool` | `false` | 跳过 SSL/TLS 证书验证 |
| `godot_cef/security/disable_web_security` | `bool` | `false` | 禁用 CORS 和同源策略 |
| `godot_cef/security/default_permission_policy` | `int` | `0` | 默认权限策略。`0` = DENY_ALL，`1` = ALLOW_ALL，`2` = SIGNAL（触发 `permission_requested`） |

### 调试设置

| 设置 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `godot_cef/debug/remote_devtools_port` | `int` | `9229` | Chrome DevTools 远程调试端口。仅在调试版本或从编辑器运行时激活。 |

### 性能设置

| 设置 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `godot_cef/performance/max_frame_rate` | `int` | `0` | 浏览器渲染的最大帧率。设为 `0` 则跟随 Godot 引擎的 FPS 设置。有效范围：1–240+。 |

### 缓存设置

| 设置 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `godot_cef/storage/cache_size_mb` | `int` | `0` | 磁盘缓存最大容量（MB）。设为 `0` 使用 CEF 默认值。 |

### 网络设置

| 设置 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `godot_cef/network/user_agent` | `String` | `""` | 自定义 User-Agent 字符串。留空则使用 CEF 默认 User-Agent。 |
| `godot_cef/network/proxy_server` | `String` | `""` | 代理服务器 URL（如 `socks5://127.0.0.1:1080` 或 `http://proxy:8080`）。留空表示直连。 |
| `godot_cef/network/proxy_bypass_list` | `String` | `""` | 不走代理的主机列表（逗号分隔，如 `localhost,127.0.0.1,*.local`）。 |
| `godot_cef/network/enable_adblock` | `bool` | `false` | 启用基于规则的请求过滤（adblock）。仅对该设置生效后新创建的浏览器实例生效（adblock 配置在浏览器创建时确定）。 |
| `godot_cef/network/adblock_rules_path` | `String` | `""` | EasyList/ABP 兼容规则文件路径。支持 `user://` 与 `res://`。当 adblock 关闭时忽略该项。 |

### 高级设置

| 设置 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `godot_cef/advanced/custom_command_line_switches` | `String` | `""` | 自定义 CEF 命令行开关（每行一个）。以 `#` 开头表示注释。格式：`switch-name` 或 `switch-name=value`。 |

::: danger 安全警告
使用此设置可以传递任意 Chromium/CEF 命令行开关，其中部分开关会绕过浏览器安全机制（例如 `disable-web-security`、`allow-running-insecure-content`）。仅在充分了解风险、且用于本地开发或受信环境时使用；不要在生产环境中禁用安全特性。
:::
::: tip 自定义开关
自定义命令行开关可用于传递额外的 CEF/Chromium 参数。每行一个开关，以 `#` 开头的行会被忽略。示例：
- `disable-gpu-compositing`
- `enable-features=WebRTC`
- `js-flags=--max-old-space-size=4096`
:::

### 配置示例

在您的 `project.godot` 文件中：

```ini
[godot_cef]
storage/data_path="user://my-app-browser-data"
storage/cache_size_mb=512
security/allow_insecure_content=false
performance/max_frame_rate=60
network/user_agent="MyApp/1.0 (Godot Engine)"
network/proxy_server="socks5://127.0.0.1:1080"
network/proxy_bypass_list="localhost,127.0.0.1"
network/enable_adblock=true
network/adblock_rules_path="user://filters/easylist.txt"
advanced/custom_command_line_switches="disable-gpu-compositing\nenable-features=WebRTC"
```

或在创建任何 CefTexture 之前通过 GDScript 配置：

```gdscript
# 在自动加载或较早加载的脚本中
func _init():
    ProjectSettings.set_setting("godot_cef/storage/data_path", "user://custom-cef-data")
```

## URL 属性

`url` 属性带有副作用：当您从 GDScript 设置它时，浏览器会自动导航到新 URL：

```gdscript
# Navigate to a new page by setting the property
cef_texture.url = "https://example.com/game-ui"

# Read the current URL (reflects user navigation, redirects, etc.)
print("Currently at: ", cef_texture.url)
```

## 加速离屏渲染

`enable_accelerated_osr` 属性控制是否使用 GPU 加速渲染：

```gdscript
# Enable GPU-accelerated rendering (recommended for performance)
cef_texture.enable_accelerated_osr = true

# Use software rendering (fallback for unsupported platforms)
cef_texture.enable_accelerated_osr = false
```

::: tip
GPU 加速可显著提升性能，但并非所有平台都可用；当不可用时系统会自动回退为软件渲染。
:::

## 背景颜色

`background_color` 属性控制浏览器的背景色。将 alpha 设为 `0` 表示透明。

```gdscript
# 透明背景（默认）
cef_texture.background_color = Color(0, 0, 0, 0)

# 实心背景
cef_texture.background_color = Color(0.2, 0.3, 0.4, 1)
```

## 弹出策略

`popup_policy` 属性控制弹出窗口（`window.open()`、`target="_blank"` 链接）的处理方式。可在运行时更改，立即对后续弹出请求生效。

| 值 | 名称 | 行为 |
|----|------|------|
| `0` | BLOCK | 静默阻止所有弹出（默认，向后兼容） |
| `1` | REDIRECT | 在当前浏览器中导航到弹出 URL，而不打开新窗口 |
| `2` | SIGNAL_ONLY | 触发 `popup_requested` 信号，由 GDScript 决定处理方式 |

```gdscript
# 阻止所有弹出（默认）
cef_texture.popup_policy = 0

# 在同一浏览器中自动跟随弹出链接
cef_texture.popup_policy = 1

# 在 GDScript 中处理弹出
cef_texture.popup_policy = 2
cef_texture.popup_requested.connect(func(url, disposition, user_gesture):
    if user_gesture:
        cef_texture.url = url  # 跟随用户触发的弹出
)
```

::: tip
REDIRECT 策略是单浏览器场景中最简单的选项——它将 `target="_blank"` 链接变为普通导航。当需要精细控制时（例如阻止广告但允许用户触发的弹出），请使用 SIGNAL_ONLY。
:::
