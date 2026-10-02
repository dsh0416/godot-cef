# Permission integration fixture

This opt-in fixture exercises the public permission API through real CEF media
callbacks, GDScript, and a loopback HTTP page. It runs ten cases: five each for
`CefTexture` and `CefTexture2D`. It is not wired into CI.

| Case | Required behavior |
| --- | --- |
| `grant_all` | A combined camera/microphone request emits two unique ids. Granting one consumes only that decision; JavaScript and both completion signals wait for the second grant. Both ids then finish as `allowed`, and JavaScript receives two synthetic tracks. |
| `deny_one` | Denying either id immediately rejects the whole group. Both ids finish as `denied`; trying to grant the other id returns `false`; JavaScript rejects with `NotAllowedError`. |
| `timeout` | Neither request is handled. With the project timeout set to one second, both ids finish as `timed_out`, and JavaScript rejects. |
| `navigation` | Navigating while both decisions are pending finishes both ids as `navigation`. After the new local page loads, neither old id can be granted or denied. |
| `unhandled_then_listen` | Initially no `permission_requested` listener exists. The first combined request is rejected and both ids finish as `dismissed`. After connecting a listener, a second request from the same page/origin emits fresh ids and can be granted, proving that the first dismissal did not cache a permanent denial. |

Every case sets the per-instance `permission_policy` to `2` while the project
default is `0`, checks `is_permission_pending()`, the Boolean results of
`grant_permission()`/`deny_permission()`, and exactly one
`permission_request_finished(id, result)` for every emitted id. A fresh Godot
process and separate CEF profile prevent permission decisions from leaking
between cases. Timeout assertions permit 0.7–5 seconds between receiving the
request signal and the completion signal to account for event scheduling.

## Run

Build a complete addon from the same checkout using the repository toolchain:

```sh
cargo xtask bundle
node tests/permissions/run.mjs --godot /absolute/path/to/godot
```

On Windows, select the console executable from an official Godot 4.5+ archive:

```powershell
node tests/permissions/run.mjs --godot 'C:/tools/Godot_v4.5-stable_win64_console.exe'
```

Use `--addon <directory>` to exercise an unpacked release addon, `--output
<directory>` to choose a results location, or `--case CefTexture2D:navigation`
to run a single case. The addon must include its helper and matching CEF runtime,
not just the extension DLL/SO. Node needs only its built-in modules; no npm
installation is required.

The runner copies the fixture and addon into a fresh subdirectory of
`target/permission-integration/`, imports the project, and runs each selected
case. It retains `summary.json`, a result JSON and Godot log per case, and an
import log. Profiles also stay under that run directory. Exit code zero requires
every selected case to pass. Missing CEF callbacks, unsupported browser features,
script errors, and process timeouts fail the run; they are not reported as passes.

## Scope and device isolation

The HTTP server binds only to `127.0.0.1` on an ephemeral port. Pages report
results to that same origin; `CefTexture2D` does not need an IPC signal. The test
sets `use-fake-device-for-media-stream` and checks synthetic track labels, then
immediately stops returned tracks. It does **not** set
`use-fake-ui-for-media-stream`, which would bypass the permission callbacks under
test. It does not request display capture, enumerate real devices, or access a
LAN host. The local origin is a secure context for `getUserMedia` without turning
off web security.

CEF acceleration is disabled for each object and with `disable-gpu`. Godot itself
uses a small Compatibility/OpenGL window so its normal rendering loop drives
the resource's `frame_pre_draw` callback. A desktop/display is required; this
fixture does not claim headless/dummy-renderer coverage or GPU validation.

These cases cover real combined camera/microphone callbacks only. They do not
prove that CEF can trigger every generic permission type, such as local-network
access, in this harness. Permission-name/mask mapping and generic prompt
continuation behavior need the accompanying Rust callback/state tests; the
fixture deliberately makes no network probe to force those prompts.
