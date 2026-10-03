# Godot headless integration tests

This suite loads the production addon and a separate Rust test GDExtension into
an official Godot 4.5 runtime. The test addon starts from the engine main loop
only when `GDCEF_ITEST=1`; the project contains an empty scene and no GDScript
test logic. Production classes are exercised through their registered Godot
API, without linking another copy of `gdcef` into the test addon.
The Rust `xtask` runner stages the project, serves the loopback browser fixtures,
supervises Godot processes, and writes the test reports. Browser-page JavaScript
is test input executed by CEF; the harness itself runs entirely in Rust.

Linux x64 runs in the required Build workflow after the native release bundle.
An integration failure therefore fails the existing Build and Gate checks. The
job reuses that bundle, builds only `gdcef_itest`, and installs Godot through
mise using the version in `mise.toml` and generated checksums in `mise.lock`.
Xvfb supplies CEF's X11 environment; Godot runs with `--headless` and its dummy
renderer. CEF acceleration is disabled per browser and with `disable-gpu`.

## Run

Build the complete addon first, then build and run the test addon:

```sh
mise install godot
cargo xtask bundle --release --target x86_64-unknown-linux-gnu
xvfb-run -a cargo xtask integration --release --target x86_64-unknown-linux-gnu \
  --godot "$(mise which godot)"
```

Linux requires the normal CEF runtime dependencies plus `xvfb` and `xauth`. The
bundle must include its helper and matching CEF runtime; an extension SO alone
is insufficient. Integration testing does not require Node or an npm install.

For a local Windows run, use the console executable from the mise installation:

```powershell
mise install godot
$godot = (Get-ChildItem (mise where godot) -Filter '*_console.exe').FullName
cargo xtask bundle
cargo xtask integration --godot $godot
```

The GitHub backend avoids a Godot filename bug in mise 2026.10.0's Aqua
backend. Windows keeps the original executable names because the console
launcher needs its matching GUI executable. To change the engine version,
update `tools.godot.version` in `mise.toml`, run the following command, and
commit both files:

```sh
mise lock godot --platform 'linux-x64,linux-arm64,macos-x64,macos-arm64,windows-x64,windows-arm64'
```

`--addon <directory>` selects a complete production addon, `--output <directory>`
chooses the evidence directory, and `--case CefTexture2D:permission_navigation`
runs one case. `--target` must match the host because the built test addon is
loaded into that host's Godot process. CI currently executes Linux x64 only;
other platforms are not validated by this integration gate.

To reuse a test addon that has already been built:

```sh
cargo xtask integration --godot /absolute/path/to/godot \
  --test-addon target/x86_64-unknown-linux-gnu/release/libgdcef_itest.so
```

`--test-addon` skips building `gdcef_itest`. The runner stages exactly the supplied
test library and generates its host descriptor. It never discovers a test DLL/SO
from a production package. The test addon and fixtures are not included in
distributed addons.

## Cases and assertions

Every case runs separately for `CefTexture` and `CefTexture2D`, giving 14 cases:

| Case | Required behavior |
| --- | --- |
| `permission_grant_all` | A combined camera/microphone request has two unique ids. The first grant does not complete the group; the second resolves JavaScript with both synthetic tracks. |
| `permission_deny_one` | One denial rejects the entire group and both ids finish as denied; the sibling cannot subsequently be granted. |
| `permission_timeout` | With no decisions, the one-second permission deadline rejects JavaScript and finishes both ids as timed out. |
| `permission_navigation` | Navigation cancels pending decisions; stale ids cannot be granted or denied. |
| `permission_unhandled_then_listen` | An absent listener dismisses the first request. Adding a listener permits a fresh request in the same browser/origin without a cached permanent denial. |
| `js_ipc` | Preload executes before the page; eval returns an observable result; text with Unicode, binary bytes, and structured data arrive intact in JavaScript. `CefTexture` also checks the echoed messages through its public inbound IPC signals. `CefTexture2D` has no such signals, so its public outbound API is checked through page reports over HTTP. |
| `lifecycle` | Three generations each load a page, resize with DOM dimensions checked, and execute JavaScript; destruction invalidates the old Godot instance before the next browser is created. `CefTexture2D` also checks shutdown before deferred startup, retaining a shutdown resource across process frames without restarting its browser, and dropping a resource without explicit shutdown. |

Permission cases use per-instance SIGNAL policy over project DENY_ALL and check
pending, duplicate, stale-id, and exactly-once completion behavior. Only fake
camera/microphone devices are enabled; `use-fake-ui-for-media-stream` is not set
because it would bypass the callbacks under test. Returned tracks are stopped
immediately. The fixture binds to `127.0.0.1` on an ephemeral port, serves its own
pages, and isolates event history by a random run id. There is no external web
dependency during the tests.

Startup waits for a successful loopback HTTP health handshake before launching
Godot, with a two-second deadline. Only connection refusal/timeouts are retried;
an invalid health response fails immediately.

## Failure and evidence contract

The runner first imports the project and checks extension-only startup/exit
with the test opt-in explicitly disabled. Each case then gets a fresh Godot
process and CEF profile. The Rust addon has a 30-second case deadline; the outer
runner has a separate 60-second process deadline that kills the Godot/CEF
process tree. Unix uses a dedicated process group; Windows starts Godot suspended,
assigns it to a kill-on-close Job Object, then resumes it so descendants inherit
the job. Cleanup also terminates remaining descendants after a normal parent exit.

Exit code zero requires all selected cases to produce exactly one matching
`GDCEF_ITEST_RESULT` JSON report with positive checks, `passed: true`, no failures,
and a clean process exit. A success report followed by a crash, missing or
duplicate reports, panic/script/load errors, or either timeout fails the run.
Unknown/empty case selections fail before execution.

Under `target/integration/<run>/`, the runner retains `summary.json`, `junit.xml`,
and each case's `result.json` and `godot.log`, including failures. The local run
also retains staged binaries and isolated profiles. CI always uploads JSON,
JUnit and logs as `integration-linux-x64`; it does not upload profiles/binaries.

The test protocol is `GDCEF_ITEST=1` plus `GDCEF_ITEST_CLASS`, `GDCEF_ITEST_CASE`,
`GDCEF_ITEST_ORIGIN`, `GDCEF_ITEST_RUN`, and `GDCEF_ITEST_PROFILE`. The Rust runner
supplies these; manual environment configuration is normally unnecessary.

Run the harness's own failure-path checks with:

```sh
cargo test --locked -p xtask integration::
```

These guards deliberately exercise unsuccessful process exits, hung descendant
processes, missing/duplicate/invalid reports, and isolation of HTTP reports.
They run without Godot or CEF. Linux x64 CI runs them before bundling the addon;
the workspace test jobs also include them on native targets.

## Coverage boundary

Strict Godot headless execution validates real GDExtension registration, CEF
process startup, API calls, callbacks, IPC and cleanup. It does **not** validate
GPU shared textures, Godot texture uploads/readback, viewport pixels, native
window input, IME, screen/DPI behavior or desktop capture. CEF software rendering
being enabled is not evidence that Godot rendered a frame. GPU and visible
rendering behavior still require their own platform/rendering tests.

The permission scenarios were migrated from `tests/permissions`; that older
GDScript runner is retired to avoid maintaining two implementations.
