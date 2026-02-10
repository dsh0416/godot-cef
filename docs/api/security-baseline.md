# Production Security Baseline

Use this baseline for production builds unless your application has a specific exception.

## Recommended Project Settings

| Setting | Recommended Value | Reason |
|---------|-------------------|--------|
| `godot_cef/security/allow_insecure_content` | `false` | Prevent mixed HTTP/HTTPS content loading |
| `godot_cef/security/ignore_certificate_errors` | `false` | Keep TLS certificate validation enabled |
| `godot_cef/security/disable_web_security` | `false` | Preserve CORS and same-origin protections |

## Custom Command-Line Switches

Keep `godot_cef/advanced/custom_command_line_switches` empty unless absolutely needed.

Avoid security-weakening switches in production, such as:

- `disable-web-security`
- `ignore-certificate-errors`
- `allow-running-insecure-content`

## Remote DevTools

Remote DevTools is intentionally only enabled in debug/editor contexts. Do not depend on it for production workflows.

## Startup Validation

At startup, Godot CEF logs:

- warnings for insecure security settings,
- warnings for insecure custom switches,
- a production baseline summary to verify expected defaults.

