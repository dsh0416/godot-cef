# 生产环境安全基线

除非有明确需求，建议生产版本遵循以下基线配置。

## 推荐项目设置

| 设置项 | 推荐值 | 说明 |
|--------|--------|------|
| `godot_cef/security/allow_insecure_content` | `false` | 避免 HTTPS 页面加载 HTTP 混合内容 |
| `godot_cef/security/ignore_certificate_errors` | `false` | 保持 TLS 证书校验 |
| `godot_cef/security/disable_web_security` | `false` | 保留 CORS 与同源策略保护 |

## 自定义命令行开关

`godot_cef/advanced/custom_command_line_switches` 建议保持为空，除非有充分理由。

生产环境应避免以下高风险开关：

- `disable-web-security`
- `ignore-certificate-errors`
- `allow-running-insecure-content`

## 远程 DevTools

远程 DevTools 仅在调试/编辑器环境下启用，生产环境不应依赖该能力。

## 启动期校验

启动时，Godot CEF 会输出：

- 不安全配置项警告，
- 不安全自定义开关警告，
- 生产安全基线摘要日志，便于核对默认值是否符合预期。

