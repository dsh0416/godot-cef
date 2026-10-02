# Production Security Baseline

Use this baseline for production builds unless your application has a specific exception.

## Recommended Project Settings

| Setting | Recommended Value | Reason |
|---------|-------------------|--------|
| `godot_cef/security/allow_insecure_content` | `false` | Prevent mixed HTTP/HTTPS content loading |
| `godot_cef/security/ignore_certificate_errors` | `false` | Keep TLS certificate validation enabled |
| `godot_cef/security/disable_web_security` | `false` | Preserve CORS and same-origin protections |
| `godot_cef/security/default_permission_policy` | `0` (`DENY_ALL`), or `2` (`SIGNAL`) with a handler | Deny unused capabilities; prompt for capabilities your app needs |
| `godot_cef/security/permission_request_timeout_seconds` | `60` | Expire unanswered requests instead of waiting indefinitely |

Both browser types can override the project policy with `permission_policy`.
For application prompts, connect both permission signals and remove stale UI on
`permission_request_finished`. Compare the CEF requesting origin exactly;
for local network permissions it is not the target device address.
See [Permissions](./permissions.md) for a queued dialog example.

These decisions do not bypass secure-context, CORS, Permissions Policy, or OS
permission requirements. Chromium may retain decisions in the shared profile;
this interface does not promise one-time grants or clear existing grants.
[`get_permission_setting()`](./permissions.md#querying-profile-settings) reads
selected profile settings; an `allow` result does not certify effective browser
or OS authorization and should not replace checking the operation's result.

## Custom Command-Line Switches

Keep `godot_cef/advanced/custom_command_line_switches` empty unless absolutely needed.

Avoid security-weakening switches in production, such as:

- `disable-web-security`
- `ignore-certificate-errors`
- `allow-running-insecure-content`
- `enable-media-stream` (bypasses the media permission callback)

## Remote DevTools

Remote DevTools is intentionally only enabled in debug/editor contexts. Do not depend on it for production workflows.

## Startup Validation

At startup, Godot CEF logs:

- warnings for insecure security settings,
- warnings for insecure custom switches,
- a production baseline summary to verify expected defaults.

