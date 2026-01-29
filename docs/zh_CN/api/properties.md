# 属性

`CefTexture` 提供多项属性，用于配置与状态管理。

## 节点属性

| 属性 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `url` | `String` | `"https://google.com"` | 要显示的 URL。设置该属性会让浏览器导航到新地址；读取时返回当前 URL（可能因用户操作/重定向而变化）。 |
| `enable_accelerated_osr` | `bool` | `true` | 启用 GPU 加速渲染 |

## 项目设置

应用于**所有** `CefTexture` 实例的全局设置在 **项目设置 > godot_cef** 中配置。这些设置必须在任何 `CefTexture` 进入场景树之前设置。

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

### 调试设置

| 设置 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `godot_cef/debug/remote_devtools_port` | `int` | `9229` | Chrome DevTools 远程调试端口。仅在调试版本或从编辑器运行时激活。 |

### 配置示例

在您的 `project.godot` 文件中：

```ini
[godot_cef]
storage/data_path="user://my-app-browser-data"
security/allow_insecure_content=false
```

或在创建任何 CefTexture 之前通过 GDScript 配置：

```gdscript
# In an autoload or early-loading script
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

