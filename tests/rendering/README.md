# Graphical publication regression

This optional hardware test opens real Godot renderers and reads viewport pixels.
It complements the headless Rust integration suite and is not part of its CI gate.
The harness uses the production addon's public GDScript API; its browser fixture
uses `res://fixture/page.html`, with no internet or test server dependency.

Build a complete native addon first, then run with the repository's Node 26:

```powershell
cargo xtask bundle
$godot = (Get-ChildItem (mise where godot) -Filter '*_console.exe').FullName
node tests/rendering/run.ts --godot $godot --driver d3d12
```

The default matrix runs `CefTexture` and `CefTexture2D`, software and accelerated
OSR, and Godot rendering thread models 0 and 2 in separate processes. All cases
still use a real graphics backend, including software CEF cases. Select a smaller
matrix with `--class CefTexture2D --mode accelerated --thread-model 2`. Multiple
values use comma separation. `--driver vulkan` or `--driver metal` requests those
backends; unavailable drivers and accelerated fallbacks fail instead of passing.
`--addon` selects a complete prebuilt addon; `--output` selects the evidence root.
Add `--gpu-validation` to pass Godot's GPU validation switch during both import
and execution. Evidence records this request; confirm the backend's debug layer
actually initialized in the log. D3D12 requires the Windows Graphics Tools debug
layer, while Vulkan requires an installed Khronos validation layer. An absent
layer is not equivalent to a successful validation run.

The runner stages one isolated project with a copy of the addon. Each case gets
its own CEF profile and a 90-second internal / 105-second outer deadline. Godot
is launched with hidden console windows on Windows and an offscreen graphical
window. Do not add `--headless` or minimize it: drawing and viewport readback are
the behaviors being tested. A desktop session and working graphics driver are
required. Timeout handling terminates the Godot process tree.

The fixture paints a solid primary-color sentinel and eight binary stripes in
one canvas update. GDScript reads the actual rendered pixels after
`frame_post_draw`; it does not infer success from callbacks or native handles.
Every expected sequence must be visible for four consecutive samples, with no
regression to an earlier decoded sequence. Cases cover:

- Two browser generations, explicit JavaScript paints, and three size changes.
  Each resize paints a new sequence only after the page's CSS viewport matches
  the requested dimensions. Node requests account for viewport stretch and CEF's
  integer conversion from physical pixels using `devicePixelRatio`; standalone
  texture sizes already specify CSS pixels. A scaled old texture cannot pass.
  Node console evidence records requested, actual CSS, canvas, and DPI dimensions.
- Accelerated nodes retain the same externally held `Texture2DRD` wrapper across
  resizes and detach its RD resource during destruction.
- Pausing and resuming: a pausable `CefTexture` retains its displayed frame while
  an independent `CefTexture2D` continues publishing during scene pause.
- Node process disabling and reenabling, plus removal and reentry into the tree.
- Two simultaneous browsers in separate viewport regions with independent pixel
  counters. Three 32-update bursts exercise source recycling; a node suspends
  publication through the first burst while its peer keeps rendering, then must
  display the newest sequence on resume. Both counters must remain monotonic.
- Destroying the peer while the original browser continues rendering.
- Explicit resource shutdown, node destruction, and invalidated instance IDs.

Each case records `command.json`, `godot.log`, `result.json`, `assessment.json`,
and `latest.png`; pixel failures also retain `failure.png`. The run's
`summary.json` reports the requested and observed backend, thread model, actual
accelerated startup, timings, sequence checks, and any failure. A success report
followed by a crash, missing/duplicate reports, engine/script/validation errors,
or a backend fallback fails the run.

Readback synchronizes some rendering work. These tests verify rendered pixels
and lifecycle behavior under those observations; they do not prove the absence
of all GPU races, frame pacing regressions, or unsampled intermediate artifacts.
They do not currently cover native input, IME, popups, HDR, or other GPUs.
Node popup input requires native window focus, which this offscreen harness
does not guarantee. `CefTexture2D` has no accelerated popup compositor; testing
it here would require a separate feature rather than exercising publication.
