# Graphical publication regression

This optional hardware test opens real Godot renderers and reads viewport pixels.
It complements the headless Rust integration suite and is not part of its CI gate.
The harness uses the production addon's public GDScript API. The lifecycle
fixture uses `res://fixture/page.html`; the pacing and popup scenarios use an
ephemeral loopback HTTP server. None needs internet access.

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

To check sustained accelerated frame pacing with VSync enabled, run:

```powershell
node tests/rendering/run.ts --godot $godot --driver d3d12 --mode accelerated --scenario pacing --gpu-validation
node tests/rendering/run.ts --godot $godot --driver vulkan --mode accelerated --scenario pacing
```

`--scenario pacing` replaces the lifecycle sequence with a continuously animated
1280×800 browser surface. It enables VSync, caps Godot and CEF at 60 FPS, warms up
for three seconds, and measures twenty seconds without viewport pixel readbacks.
The page reports rAF intervals to the runner's loopback server; Godot independently
records its actual draw count and one-second samples. Both rates must reach 80%
of `min(60, display refresh rate)` (60 Hz if refresh discovery is unavailable),
and the browser/Godot rate ratio must be at least 0.8. The browser's final ten
seconds must also meet the rate threshold, catching degradation after warmup.
The longer sample covers a full frame of phase drift between 60 and 59.94 Hz;
at least nineteen complete one-second browser samples are required.
Run this scenario on an otherwise idle desktop with VSync available; it is a
hardware performance regression, not a portable headless timing assertion.

The pacing scenario checks rendered sentinel pixels before and after sampling,
waiting for the browser's report before resuming readbacks. It then exits with
the browser still animating to exercise completion callbacks during scene
shutdown. Missing or duplicate reports, low rates, native errors, and leaked RID
warnings fail the run. `assessment.json` and `summary.json` retain both sets of
timings, the refresh baseline, thresholds, and computed ratio.

To check native popup input and GPU composition after navigation, run:

```powershell
node tests/rendering/run.ts --godot $godot --driver d3d12 --mode accelerated --scenario popup --gpu-validation
node tests/rendering/run.ts --godot $godot --driver vulkan --mode accelerated --scenario popup
```

`--scenario popup` redirects a focused browser from `localhost` to `127.0.0.1`
before opening a real HTML select menu. This exercises focus restoration when
Chromium replaces its render widget during navigation. The fixture focuses the
native window but never blurs and refocuses the Godot control after navigation:
that workaround would hide the focus-cache regression found on TestUFO.

The fixture sends native mouse and keyboard events through each class's public
input path. Actual viewport pixels must show the menu, confirm selection of TWO,
hide on dismissal, reopen with the same native popup dimensions, and show a moved
and resized menu. The externally retained texture RID must remain stable.
Midtone RGB, half-transparent content, and a fully transparent area over a known
scene background must remain unchanged while composition is active. `CefTexture`
uses its child overlay; `CefTexture2D` uses a GPU canvas viewport behind a stable
texture proxy. Browser frames are never read back to implement composition.

Popup results retain `popup-before.png` and one PNG per checked transition. The
case exits with the popup still visible to exercise compositor and texture
cleanup. This scenario checks a fixed-size viewport; it does not certify all
platform-native menu metrics, nested viewport render ordering, IME, or menus
outside a standalone texture's fixed bounds. Node popups can extend outside the
control; standalone texture popups are clipped to the texture dimensions.

The runner stages one isolated project with a copy of the addon. Each case gets
its own CEF profile and a 90-second internal / 105-second outer deadline (the
popup scenario has a 60-second internal deadline). Godot
is launched with hidden console windows on Windows and an offscreen graphical
window. Do not add `--headless` or minimize it: drawing and viewport readback are
the behaviors being tested. A desktop session and working graphics driver are
required. Timeout handling terminates the Godot process tree.

The default `--scenario lifecycle` fixture paints a solid primary-color sentinel and eight binary stripes in
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
and scenario-specific PNGs (`latest.png` and `failure.png` for lifecycle). The run's
`summary.json` reports the requested and observed backend, thread model, actual
accelerated startup, timings, sequence checks, and any failure. A success report
followed by a crash, missing/duplicate reports, engine/script/validation errors,
or a backend fallback fails the run.

Lifecycle readback synchronizes some rendering work, so use the separate pacing
scenario to test sustained rendering without that synchronization. Passing these
scenarios does not prove the absence of all GPU races or unsampled artifacts.
The lifecycle and pacing fixtures do not cover native input; the separate popup
scenario covers select-menu interaction only. IME, HDR, and untested GPUs remain
outside these checks. Popup input requires a focused native window even when its
position is offscreen.
