# Accelerated frame handoff

`CefTexture` and `CefTexture2D` publish browser textures from `RenderingServer.frame_pre_draw`.
The signal runs on the main thread; it queues publication through `call_on_render_thread`.
It is not a GPU fence. Browser lifecycle, CEF message pumping, events, and sizing
continue through process callbacks, including when Godot is not drawing.

## Ownership and completion

Each browser has separate view and popup streams. Each stream contains three
persistent snapshot slots, with at most two resource generations during resize.
Dimensions and BGRA/RGBA format are part of a generation. Frames with no available
slot are skipped before any borrowed CEF storage is read; the last display stays visible.
Skipped paints request a fresh view/popup paint from the main thread until capture
succeeds, so a static page can recover after bootstrap, resize, or pool exhaustion.

1. RD creates and initializes staging, copies a pixel into a per-slot sentinel, and
   reads that sentinel asynchronously. Only its completion makes the slot available.
   Staging now has a known copy-source state in both Godot and the native backend.
   A one-time zero upload initializes empty resources without requiring sRGB
   render-target/UAV clear support; browser frames stay on the GPU.
2. `OnAcceleratedPaint` opens the current borrowed source, captures into a free
   slot, restores its copy-source state, and waits for all borrowed-source GPU
   access to finish before returning. Handles, IOSurfaces and DMA-BUF identities
   are not used to cache a borrowed frame.
3. Before drawing, the rendering thread selects the newest completed snapshot
   and records `RD.texture_copy(staging, display)`. Godot tracks display copy and
   sampling dependencies.
4. A copy from **display** to the slot's sentinel, followed by
   `texture_get_data_async(sentinel)`, proves completion of the publication copy.
   Only the matching generation/use token can release its staging slot.

Sentinel copies directly from staging would be independent reads and would not
order publication. Frame counts, `frame_post_draw`, CPU call completion and
timeouts are never accepted as GPU completion. Failed completion paths quarantine
resources until device teardown. Display replacement is bound before old resources
are retired; outstanding readback callbacks retain their own generation.

## Native backend contracts

| Backend | Capture and handoff |
| --- | --- |
| Windows D3D12 | Open the CEF shared handle directly with D3D12 inside the callback. Capture on an owned queue and acquire/release the source through COMMON. CPU-observed slot completion proves earlier Godot reads have finished; the capture fence orders publication on Godot's queue and ends borrowed-source access before callback return. Preserve RD's copy-source state, bridging enhanced and legacy barriers through COMMON. |
| Windows Vulkan | Reopen external Win32 memory for each callback. Capture on a reserved private queue in Godot's family when available, otherwise use Godot's queue. Acquire/release external image ownership, restore staging to `TRANSFER_SRC_OPTIMAL`, and wait for the native copy fence before returning. A semaphore wait on Godot's queue orders publication after private-queue capture. |
| Linux Vulkan | Import current DMA-BUF planes and DRM modifier, acquire producer readiness through an exported sync-file semaphore (or native DMA-BUF fence polling on older kernels), transfer foreign ownership for capture and release it before callback return. Use the actual Godot queue/family and completed native copy. |
| macOS Metal | Open the current IOSurface and blit into a tracked staging texture on Godot's own thread-safe `MTLCommandQueue`. Same-queue hazard tracking supplies visibility; `waitUntilCompleted` ends all borrowed-source access before return. |

Vulkan interception serializes host access to the real queue, including Godot's
background transfer work. Initialization verifies device and queue provenance;
merely running on the render thread does not establish exclusive queue access.
Queue wrappers stay loaded for the process lifetime because the driver caches
their addresses. Restart Godot after rebuilding the extension.

On Windows, the device-creation hook can append one queue when the physical
family has spare capacity, without changing Godot's queue indices or priorities.
The importer uses it only when successful creation and queue provenance are
recorded; without a reservation it retains the existing shared-queue path. Private capture
signals a semaphore, and an empty submission on Godot's queue waits at the
transfer stage before subsequent RD copies. A separate fence retires that wait;
normal private capture does not synchronously wait for this Godot-queue fence.
Linux continues to capture on Godot's actual queue.

Godot 4.6+ exposes the native D3D12 texture and command queue directly. The
importer verifies the queue's device identity before submitting capture work.
An available D3D12 slot already has completion proof from its bootstrap or
previous publication. Adding a new Godot-to-capture fence behind unrelated
rendering work would make capture wait unnecessarily and can reduce browser
frame rate under VSync. The capture-to-Godot fence remains in place.

## Popup presentation

`CefTexture` displays an accelerated popup in a child overlay, allowing it to
extend beyond the node's bounds, subject to normal ancestor clipping.
`CefTexture2D` keeps a stable public RenderingServer texture proxy: it points to
the native view normally and to a GPU canvas viewport combining view and popup
while a popup is visible. The canvas uses premultiplied alpha and clips popups to
the texture's bounds. Opening, hiding, or resizing the popup preserves the public
texture RID. This composition uses GPU textures without CPU readback or software
fallback.

## Runtime errors and validation

Browser initialization preserves the existing software fallback when acceleration
is unavailable or the native importer cannot be created; the fallback reason is
logged. Setting `enable_accelerated_osr = false` also uses software rendering.

After accelerated rendering starts, capture or publication failures retain the
last successfully published frame. Incomplete GPU work keeps its resources
quarantined until device teardown.

Run the graphical pixel/lifecycle suite described in
[`tests/rendering`](https://github.com/dsh0416/godot-cef/tree/main/tests/rendering).
It verifies the requested renderer and thread model. Accelerated test cases reject
startup fallback so software pixels cannot pass a GPU test; engine errors, stale
pixel sequences, crashes or timeouts also fail. Headless tests
verify runtime progression but cannot validate GPU sharing or displayed pixels.

The `pacing` scenario enables VSync, warms up for three seconds, then measures
twenty seconds of CEF rAF and Godot draws without viewport readback. It checks
sustained rates, including the final ten seconds, before quitting with an
active browser. The `popup` scenario uses native mouse and keyboard input after
a cross-site redirect to check focus preservation, popup open/select/reopen/close,
changed popup bounds, stable texture identity, and preserved colors and alpha.

The minimum supported Godot version is 4.6. Passing compilation does not
replace graphical validation on each OS, GPU driver, and Godot version. This work
continues [#227](https://github.com/dsh0416/godot-cef/issues/227).
