# Permissions

`CefTexture` and `CefTexture2D` expose the same permission controls for camera,
microphone, local network access, and other permission requests delivered by CEF.
Your application can show a Godot dialog and answer each request asynchronously.

## Policy and lifetime

Set `permission_policy` on the browser:

| Value | Behavior |
|-------|----------|
| `-1` (default) | Inherit `godot_cef/security/default_permission_policy` |
| `0` | Deny requests |
| `1` | Allow known permission requests without an application prompt |
| `2` | Emit `permission_requested` for an application decision |

The project default is `0`. Connect the signals before setting the policy to `2`
or loading content that requests permissions. A request without a connected
`permission_requested` listener is canceled without authorization, with a
`dismissed` finished result. This dismisses a CEF prompt without recording an
explicit site denial. Only that request's permission group is canceled; unrelated
groups already delivered to the application remain pending. Unknown permission bits are denied,
including under policy `1`.

`godot_cef/security/permission_request_timeout_seconds` sets the response deadline
(default `60` seconds, minimum `1`). Configure it before browser creation.
An unanswered request expires at the deadline. Timeout processing follows
Godot's main loop, so a blocked main thread delays notification; answer methods
still check the actual deadline and reject late answers. Changing
`permission_policy` at runtime cancels pending requests. Navigation, browser
closure, and renderer termination invalidate outstanding requests as well.

For CEF permission prompts, timeout and lifecycle cancellation dismiss the prompt
instead of recording an explicit deny decision. Explicit rejection uses CEF's
deny result. Media requests are canceled for timeouts and lifecycle changes, and
denied when explicitly rejected. Neither action clears
permission decisions that Chromium has already stored.

CEF can combine several permissions into one request. The signal is emitted once
per permission type, with a separate `request_id`, but **the decision is atomic**:
all IDs must be granted to allow the CEF request; denying one ID rejects the whole
request. In particular, a camera-and-microphone request cannot be granted only
camera access. `grant_permission()` returning `true` means that answer was
accepted, not that every permission in the group has been granted.

## Signals and methods

| API | Meaning |
|-----|---------|
| `permission_requested(permission_type: String, url: String, request_id: int)` | Ask the application to decide |
| `permission_request_finished(request_id: int, result: String)` | Close the application's pending UI for this ID |
| `grant_permission(request_id: int) -> bool` | Record an allow decision |
| `deny_permission(request_id: int) -> bool` | Deny the request and its grouped permissions |
| `is_permission_pending(request_id: int) -> bool` | Whether this ID still accepts an answer |
| `get_permission_setting(permission_type: String, requesting_url: String, top_level_url: String) -> String` | Read the current profile content setting for a supported permission |

Answer methods return `false` for expired, unknown, invalidated, or already
answered IDs. Do not reuse IDs or treat them as persistent permission records.

When the whole CEF request finishes, every ID in that group receives the same
finished result, including IDs whose allow answer was recorded earlier.
The finished `result` is one of `allowed`, `denied`, `dismissed`, `timed_out`,
`navigation`, `browser_closed`, `renderer_terminated`, or `policy_changed`.
An `allowed` result records completion of the permission decision; it does not
guarantee that the requested device or network operation succeeds.

Finished signals are delivered while the Godot object remains alive. Explicit
`CefTexture2D.shutdown()` can report `browser_closed`, but freeing a node or
resource does not guarantee a final signal. Also clear dialog state when its
browser/owner is freed; do not depend on a final signal from a destroyed object.

The `url` argument is the **requesting origin reported by CEF**, not a download
URL, the destination LAN address, or the top-level page URL. Compare trusted
origins exactly as serialized by CEF, including scheme, port, and the trailing
slash for HTTP(S) origins (for example, `https://trusted.example/`). A string-prefix check would also
match unwanted hosts such as `https://trusted.example.attacker.test`.

## Querying profile settings

Both browser types provide a synchronous, read-only query:

```gdscript
var setting: String = browser.get_permission_setting(
    "local_network", "https://app.example/", "https://app.example/"
)
print("Local network profile setting: ", setting)
```

Call it on Godot's main thread after the browser is ready. `requesting_url` is
CEF's primary URL and `top_level_url` is its secondary URL, identifying the
requesting page and the top-level page respectively. For a top-level request,
pass the same page URL twice. For an embedded page, supply both URLs explicitly;
the permission signal only supplies the requesting origin. Chromium's matching
rules depend on the setting type; storage-access settings can depend on both
sites. A local network query uses the page URLs, not the destination device's
IP address.

Both arguments must be absolute HTTP(S) URLs with a host and no username or
password. Paths, queries, and fragments are accepted and interpreted by CEF.
Empty strings, relative URLs, `res://`, `user://`, `file://`, and other schemes
return `invalid_url`; the method does not infer a URL from the current page.

| Result | Meaning |
|--------|---------|
| `allow` | The applicable profile content setting is allow |
| `block` | The applicable profile content setting is block |
| `ask` | The applicable profile content setting is ask |
| `default` | CEF returned its default/no-value sentinel; this is not an alias for `ask` |
| `session_only` | CEF returned a session-only content setting |
| `unknown` | CEF returned another content-setting value |
| `unsupported` | This permission label has no supported scalar query mapping |
| `unavailable` | The browser/request context is unavailable, or the call is outside CEF's UI thread |
| `invalid_url` | At least one URL does not meet the requirements above |

The lookup uses the browser's shared request context, including applicable
profile defaults. It does not tell you whether the value came from an explicit
site decision or a default, or how long it will persist. It leaves
`permission_policy` and pending requests unchanged and does not show a prompt.

**A profile setting is not a guarantee that an operation is currently allowed.**
It does not combine application policy, secure-context and Permissions Policy
checks, operating-system permissions, or device availability. In particular,
CEF's camera/microphone request callback can allow capture without updating the
profile setting, so a successful `grant_permission()` need not make this query
return `allow`. Use the operation's own result to determine whether it succeeded.

The supported query labels are `ar_session`, `camera_pan_tilt_zoom`, `camera`,
`captured_surface_control`, `clipboard`, `top_level_storage_access`, `local_fonts`,
`hand_tracking`, `idle_detection`, `microphone`, `midi_sysex`, `notifications`,
`keyboard_lock`, `pointer_lock`, `storage_access`, `vr_session`,
`web_app_installation`, `window_management`, `local_network`, `loopback_network`,
and `sensors`. `clipboard` queries the read/write setting, not every clipboard
operation. Defaults differ by type; for example, `sensors` may return `allow`
without an earlier application prompt.

Other labels return `unsupported`, even if they can appear in
`permission_requested`. This includes `geolocation` (CEF 154 can use structured
geolocation settings), deprecated `local_network_access`, desktop capture,
file access, and permissions without one portable scalar setting. The query
does not expose arbitrary CEF content-setting types or clear stored decisions.

The underlying contracts are documented in the pinned
[CEF request-context API](https://github.com/chromiumembedded/cef/blob/564dd6c/include/cef_request_context.h#L240)
and [Chromium permission mapping](https://github.com/chromium/chromium/blob/154.0.8037.58/components/permissions/request_type.cc#L383).

## Queueing a Godot permission dialog

Add a `CefTexture` and a `ConfirmationDialog` as children of this `Control`.
The example shows one request at a time and removes stale requests when the
browser finishes or invalidates them. It also works with a `CefTexture2D`
reference in place of the node's `browser` reference.

```gdscript
extends Control

@onready var browser: CefTexture = $CefTexture
@onready var dialog: ConfirmationDialog = $ConfirmationDialog

var pending: Array[Dictionary] = []
var active_id: int = -1

func _ready() -> void:
    dialog.ok_button_text = "Allow"
    dialog.cancel_button_text = "Deny"
    dialog.confirmed.connect(_answer.bind(true))
    dialog.canceled.connect(_answer.bind(false))
    browser.permission_requested.connect(_on_permission_requested)
    browser.permission_request_finished.connect(_on_permission_finished)
    browser.permission_policy = 2

func _on_permission_requested(kind: String, origin: String, id: int) -> void:
    pending.append({"id": id, "kind": kind, "origin": origin})
    _show_next.call_deferred()

func _show_next() -> void:
    if active_id != -1 or not is_instance_valid(browser):
        return
    while not pending.is_empty():
        var request: Dictionary = pending.pop_front()
        var id: int = request["id"]
        if not browser.is_permission_pending(id):
            continue
        active_id = id
        dialog.dialog_text = "%s requests %s. Allow?" % [
            request["origin"], request["kind"]
        ]
        dialog.popup_centered()
        return

func _answer(allow: bool) -> void:
    var id: int = active_id
    active_id = -1
    dialog.hide()
    if is_instance_valid(browser) and browser.is_permission_pending(id):
        if allow:
            browser.grant_permission(id)
        else:
            browser.deny_permission(id)
    # Wait until this dialog's button event has finished before showing another.
    _show_next.call_deferred()

func _on_permission_finished(id: int, _result: String) -> void:
    for index in range(pending.size() - 1, -1, -1):
        if pending[index]["id"] == id:
            pending.remove_at(index)
    if active_id == id:
        active_id = -1
        dialog.hide()
    _show_next.call_deferred()
```

Keep this dialog under the same owner as the browser. If your application frees
the browser separately, hide the dialog and clear `pending` and `active_id` in
that teardown path as well.

For an origin allowlist, deny requests from origins outside your exact allowlist
before adding them to `pending`. Keep the dialog for permissions that still
require the user's choice.

## Permission type names

These are bindings for the CEF version shipped by this project. Some types are
platform-specific or require Chrome-style browser features; a named binding
does not imply every web API is available on every platform.

| Category | `permission_type` values |
|----------|--------------------------|
| Local connections | `local_network`, `loopback_network` |
| Legacy local connections | `local_network_access` (deprecated CEF permission bit) |
| Camera and microphone | `camera`, `microphone`, `camera_pan_tilt_zoom` |
| Screen capture | `desktop_audio_capture`, `desktop_video_capture`, `captured_surface_control` |
| Location and devices | `geolocation`, `midi_sysex`, `sensors`, `idle_detection` |
| Clipboard and notifications | `clipboard`, `notifications` |
| Input and windows | `keyboard_lock`, `pointer_lock`, `window_management` |
| Storage and files | `storage_access`, `top_level_storage_access`, `disk_quota`, `file_system_access`, `multiple_downloads` |
| XR | `ar_session`, `vr_session`, `hand_tracking` |
| Other | `local_fonts`, `identity_provider`, `protected_media_identifier`, `register_protocol_handler`, `web_app_installation` |
| Unrecognized | `unknown_permission`, `unknown_media_permission` (cannot be granted) |

In signal mode, an unrecognized permission can be reported for diagnostics, but
attempting to grant it completes its entire CEF request as `denied`.

`local_network` refers to local network access; `loopback_network` refers to
services on the same computer, such as `127.0.0.1` or `::1`. These permissions
are separate from camera access and from ordinary CORS checks. Granting a local
network permission does not disable CORS, TLS validation, or other browser checks.

## Browser and operating-system boundaries

- Camera/microphone and local network web APIs still require an eligible secure
  context and applicable Permissions Policy. Cross-origin iframe access may need
  explicit delegation from the embedding page. Granting a request cannot make an
  ineligible page eligible.
- The operating system may separately require camera, microphone, screen capture,
  or local network permission. For example, macOS applications need appropriate
  usage descriptions and OS authorization. This API answers CEF's request; it
  does not grant OS permission or select capture devices.
- Chromium may remember prompt decisions in the current browser profile. This
  interface does not guarantee an allow decision applies only once and does not
  clear previous decisions. The profile query above reads selected settings;
  it does not enumerate or manage a persistent permission store.
  An existing decision may prevent a new prompt from reaching this API.
- Avoid `enable-media-stream` when using application-mediated media permissions:
  CEF documents that this switch bypasses its media permission callback.

See the [security baseline](./security-baseline.md), the pinned
[CEF permission contract](https://github.com/chromiumembedded/cef/blob/564dd6c/include/cef_permission_handler.h),
[CEF prompt implementation](https://github.com/chromiumembedded/cef/blob/564dd6c/libcef/browser/permission_prompt.cc),
[web media permission requirements](https://www.w3.org/TR/mediacapture-streams/#permissions-policy-integration),
and [Chrome's local network access explanation](https://developer.chrome.com/blog/local-network-access).
