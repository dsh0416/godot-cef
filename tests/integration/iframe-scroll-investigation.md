# Cross-site iframe wheel investigation (#286)

Investigation date: 2026-11-15. Related issue:
[godot-cef #286](https://github.com/dsh0416/godot-cef/issues/286).

The `iframe_scroll` integration scenario compares the same public Godot wheel
input against a same-site child and a `localhost` child of a `127.0.0.1` parent.
It first checks ordinary scrolling, then hides and restores the scroller with
`display: none`, and finally checks cancellation with `preventDefault()`.
The child reports actual trusted DOM wheel events and the resulting scroll
offset through `postMessage`; the parent forwards reports to the fixture server.
The fixture does not disable site isolation, inject synthetic DOM wheel events,
or script-scroll after restoring the element. Scripted scrolling at that point
can itself mask the reported failure.

After assembling the addon, select the diagnostic explicitly:

```sh
cargo xtask integration --case CefTexture:iframe_scroll --godot <godot-executable>
cargo xtask integration --case CefTexture2D:iframe_scroll --godot <godot-executable>
```

Use `--addon <separate-addon-directory>` to compare an isolated runtime bundle.
A failure reports the browser's received wheel event and actual scroll offset;
it must not be interpreted as an expected passing test for the affected runtime.

## Windows runtime comparison

The reproduction was run with Godot 4.6 stable, strict headless execution,
software OSR, and the unmodified production addon built from `22894aa`.
An isolated copy of that addon was then run with the official CEF 158.0.3 beta
runtime. SHA-256 checks confirmed that `gdcef.dll` and `gdcef_helper.exe` were
identical in both runs. Both runtimes advertise the same Windows API 15600 hash,
`8f211699a44daedd96ae2a89bbf3a8f2abc229f3`, used by the Rust bindings. Import and
startup checks passed with the beta.

| Scenario | CEF 156.0.26, both browser types | CEF 158.0.3 beta, both browser types |
| --- | --- | --- |
| Same-site child, initial wheel | 100 px | 100 px |
| Same-site child, hide/show then wheel | 100 px | 100 px |
| Cross-site child, initial wheel | 100 px | 100 px |
| Cross-site child, hide/show then wheel | **0 px** | 100 px |
| Cross-site child, another hide/show then wheel | Not reached after failure | 100 px |
| Cross-site child, cancelled wheel | Not reached after failure | 0 px |

Every observed wheel had `isTrusted=true`, `deltaY=100`, and the expected content
target. The failing stable event was not default-prevented. The beta completed
all six phases: 34 checks for `CefTexture` and 32 for `CefTexture2D`.
This confirms the reported minimal reproduction and shows that the beta runtime
resolves it in this environment. The runtime comparison does not isolate one
commit or establish coverage for the reporter's full application, other operating
systems, GPU rendering, or physical trackpad input.

Local retained reports from this investigation:

- Stable `CefTexture`: `target/integration/1794680187417-89acbf1363dfbf80642cbbc417504dfa/summary.json`
- Stable `CefTexture2D`: `target/integration/1794680223107-a0381579d1d5d911be0695939489ed26/summary.json`
- Beta `CefTexture`: `target/beta-integration/1794680162662-28341efc15de4b841d6dc7a26a161556/summary.json`
- Beta `CefTexture2D`: `target/beta-integration/1794680176269-87799830c28cdcbce7e970fa0c0705d3/summary.json`

## Native routing finding

The locked dependency is `cef = 156.4.0+156.0.26`, using CEF revision `8ea0c73`.
Its [OSR wheel implementation](https://github.com/chromiumembedded/cef/blob/8ea0c73/libcef/browser/osr/render_widget_host_view_osr.cc)
passes `false` to both
`SendWheelEndForTouchpadScrollingIfNeeded` and
`AddPhaseIfNeededAndScheduleEndEvent`, then routes the actual wheel input using
`ShouldRouteEvents()` and `RenderWidgetHostInputEventRouter`.

In Chromium's
[MouseWheelPhaseHandler](https://github.com/chromium/chromium/blob/156.0.8078.0/content/browser/renderer_host/input/mouse_wheel_phase_handler.cc),
that boolean decides whether the synthetic wheel-end event goes through the
input router. With `false`, it is sent directly to the root view. An
out-of-process child can therefore receive the wheel updates without their
matching routed wheel-end event. This is a plausible cause of stale scrolling
state when an iframe removes and recreates a scrollable layout object. It is
not yet proof that correcting the routing fixes every case described in #286.

CEF commit
[bca5a535](https://github.com/chromiumembedded/cef/commit/bca5a535772460e836c0618ed0c4a4ac911ea75b)
changes both arguments to `ShouldRouteEvents()` as part of its unified wheel
input API work. The commit also changes precise-device/momentum handling; the
two routing arguments can be investigated separately in a CEF 156 backport.

## Available releases

The official [CEF binary index](https://cef-builds.spotifycdn.com/index.json)
and [Rust crate release metadata](https://crates.io/api/v1/crates/cef) were
checked on the investigation date:

| Distribution | Version | Wheel-end routing |
| --- | --- | --- |
| Locked Rust crate/runtime | `156.4.0+156.0.26`, CEF `8ea0c73` | Root-only |
| Latest published Rust crate | `156.4.0+156.0.26` | Root-only |
| Latest stable CEF binary | `156.0.30+g6028375`, Chromium `156.0.8078.96` | Root-only |
| Available beta CEF binary | `158.0.3+g8c29be2`, Chromium `158.0.8127.18` | Routed |

The [stable 8078 branch](https://github.com/chromiumembedded/cef/blob/6028375/libcef/browser/osr/render_widget_host_view_osr.cc)
still has the original routing, while the
[beta 8127 branch](https://github.com/chromiumembedded/cef/blob/8c29be2/libcef/browser/osr/render_widget_host_view_osr.cc)
contains the change. The runtime comparison above verifies the beta's behavior
for this reproduction without changing Godot CEF's dependency pins.

## Integration boundary

Godot CEF's wheel path calls the public `CefBrowserHost::SendMouseWheelEvent`
API. CEF 156 does not expose the wheel-phase handler or a method to route its
pending end event. The inspected Chromium 156 phase handler applies latching
unconditionally; it provides no supported command-line switch to disable this
behavior. Disabling site isolation changes the security model and is not a
fix.

This repository downloads CEF binaries; it does not build the CEF browser
implementation. A native backport therefore requires rebuilding CEF 156 and
using the resulting matching runtime and native support artifacts. Alternatively,
a future dependency update can adopt a compatible released runtime containing
the change. Testing the available beta also requires checking its versioned API,
sandbox compatibility, packaging, and platform support; it is not an ordinary
Rust source-only fix. The isolated Windows run above confirms the selected API
works there; it does not qualify the beta as a production dependency across
all supported platforms.

A delayed JavaScript `scrollBy()` fallback is intentionally not installed by
default. Lack of observable scrolling after a fixed number of animation frames
does not prove that the native default action was lost: delayed compositor
feedback, scroll snapping, page handlers, and nested scroll chains can all make
that assumption wrong. Such a fallback can produce extra scrolling and alter
page behavior even when site isolation is retained.
