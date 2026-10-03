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
| Windows D3D12 | Open the CEF shared handle directly with D3D12 inside the callback. Capture on an owned queue, acquire/release the source through COMMON, use release/capture fences in both queue directions, and wait for capture completion. Preserve RD's copy-source state, bridging enhanced and legacy barriers through COMMON. |
| Windows Vulkan | Reopen external Win32 memory for each callback. Use Godot's actual queue/family, acquire/release external image ownership, and restore staging to `TRANSFER_SRC_OPTIMAL`. Wait for the native copy fence. |
| Linux Vulkan | Import current DMA-BUF planes and DRM modifier, acquire producer readiness through an exported sync-file semaphore (or native DMA-BUF fence polling on older kernels), transfer foreign ownership for capture and release it before callback return. Use the actual Godot queue/family and completed native copy. |
| macOS Metal | Open the current IOSurface and blit into a tracked staging texture on Godot's own thread-safe `MTLCommandQueue`. Same-queue hazard tracking supplies visibility; `waitUntilCompleted` ends all borrowed-source access before return. |

Vulkan interception serializes host access to the real queue, including Godot's
background transfer work. Initialization verifies device and queue provenance;
merely running on the render thread does not establish exclusive queue access.
Queue wrappers stay loaded for the process lifetime because the driver caches
their addresses. Restart Godot after rebuilding the extension.

Godot 4.6+ exposes the native D3D12 texture and command queue directly. The
importer verifies the queue's device identity before submitting capture work.

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

The minimum supported Godot version is 4.6. Passing compilation does not
replace graphical validation on each OS, GPU driver, and Godot version. This work
continues [#227](https://github.com/dsh0416/godot-cef/issues/227).
