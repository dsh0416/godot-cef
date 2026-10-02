# 生产环境安全基线

除非有明确需求，建议生产版本遵循以下基线配置。

## 推荐项目设置

| 设置项 | 推荐值 | 说明 |
|--------|--------|------|
| `godot_cef/security/allow_insecure_content` | `false` | 避免 HTTPS 页面加载 HTTP 混合内容 |
| `godot_cef/security/ignore_certificate_errors` | `false` | 保持 TLS 证书校验 |
| `godot_cef/security/disable_web_security` | `false` | 保留 CORS 与同源策略保护 |
| `godot_cef/security/default_permission_policy` | `0`（`DENY_ALL`），或已连接处理器的 `2`（`SIGNAL`） | 拒绝不使用的能力，对应用需要的能力显示提示 |
| `godot_cef/security/permission_request_timeout_seconds` | `60` | 使未回答的请求过期，避免无限等待 |

两种浏览器类型均可通过 `permission_policy` 覆盖项目策略。
显示应用权限提示时，请连接两个权限信号，并通过 `permission_request_finished`
清理失效界面。应精确比较 CEF 提供的请求来源 origin；对于局域网权限，
它不是目标设备地址。排队显示对话框的完整示例见[权限](./permissions.md)。

授权不能绕过安全上下文、CORS、Permissions Policy 或操作系统权限要求。
Chromium 可能在共享 profile 中保留决定；此接口不保证一次性授权，也不清除已有授权。
[`get_permission_setting()`](./permissions.md#查询配置中的权限设置) 读取部分配置值；
返回 `allow` 不保证浏览器或操作系统实际允许操作，应检查操作本身的结果。

## 自定义命令行开关

`godot_cef/advanced/custom_command_line_switches` 建议保持为空，除非有充分理由。

生产环境应避免以下高风险开关：

- `disable-web-security`
- `ignore-certificate-errors`
- `allow-running-insecure-content`
- `enable-media-stream`（会绕过媒体权限回调）

## 远程 DevTools

远程 DevTools 仅在调试/编辑器环境下启用，生产环境不应依赖该能力。

## 启动期校验

启动时，Godot CEF 会输出：

- 不安全配置项警告，
- 不安全自定义开关警告，
- 生产安全基线摘要日志，便于核对默认值是否符合预期。

