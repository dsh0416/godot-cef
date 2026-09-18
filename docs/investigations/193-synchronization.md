# Rendering and input synchronization investigation

Investigated 2026-09-19 against `e78587f43863d4b454461148cbfc8f1e664389df`
(v1.16.0). The dependency lock resolves `cef` and `cef-dll-sys` to
`152.3.0+152.0.6`; the underlying CEF revision is
`708dc140cbc3286826a8abef89dc23a44ff9ea72`, Chromium `152.0.7977.83`.
Godot bindings target API 4.5.

This records the baseline diagnosis and implementation design; local source
line numbers refer to the revision above. The Windows reports have not been
reproduced in this investigation. Confirmed code/API violations are
distinguished from explanations of the reported symptoms.

The GPU work is tracked in [#227](https://github.com/dsh0416/godot-cef/issues/227).
The agreed delivery scope is separate drag, focus/input and scheduling fixes.
Native GPU hook implementation is deferred until a manual design decision.

## Conclusion

The durable solution has three parts: capture immutable application-owned
frames inside CEF's paint callback, publish them through a GPU handoff that
Godot understands, and give focus/drag interactions explicit lifecycle owners.
A process-wide CEF scheduler should service these operations independently of
individual texture nodes.

One frame of asynchronous input-to-display latency does not itself explain
reverting pixels, stolen focus, or an indefinitely active drag. These are
separate correctness failures that share poorly defined ownership boundaries.

| Report | Evidence in the current implementation | Confidence |
| --- | --- | --- |
| [#181](https://github.com/dsh0416/godot-cef/issues/181): old/new page states alternate during scrolling | CEF resources are read after their callback lease ends; native destination writes bypass Godot's resource tracking | Contract violations confirmed; attribution of the video needs reproduction |
| [#193](https://github.com/dsh0416/godot-cef/issues/193): input/texture synchronization | Source lifetime, destination synchronization, and update-before-pump scheduling are separate problems | Confirms the need for a systematic change; does not establish a single IPC latency bug |
| [#183](https://github.com/dsh0416/godot-cef/issues/183): focus switching glitches | Repeated IME proxy focus transfer can leave CEF blurred; IME deactivation can reclaim focus from another control | Concrete state-transition defects; Windows/IME validation required |
| [#207](https://github.com/dsh0416/godot-cef/issues/207): repeated selection/drag freezes | `StartDragging` accepts every request without guaranteeing completion, including when no handler consumes the event | Contract violation confirmed; plausible explanation for the second drag sticking |

## 1. The source texture is borrowed, not a snapshot

The pinned [CEF callback contract](https://github.com/chromiumembedded/cef/blob/708dc140cbc3286826a8abef89dc23a44ff9ea72/include/cef_render_handler.h#L151-L167)
requires reopening the resource for each callback and copying its contents into
application-owned storage. It prohibits caching/access outside the callback.
The [CEF consumer](https://github.com/chromiumembedded/cef/blob/708dc140cbc3286826a8abef89dc23a44ff9ea72/libcef/browser/osr/video_consumer_osr.cc#L15-L22)
calls `Done()` as its scoped frame callback exits, releasing the pool reservation.

Current path:

```text
OnAcceleratedPaint(frame N)
  -> duplicate HANDLE / dup DMA-BUF fd / retain IOSurface
  -> save pending handle
  -> return: Chromium may reuse the frame's storage
Godot update_texture()
  -> reopen/import saved handle
  -> copy whatever pixels occupy that storage now
```

The deferred operation is explicit in
[`AcceleratedRenderHandler::on_accelerated_paint`](https://github.com/dsh0416/godot-cef/blob/e78587f43863d4b454461148cbfc8f1e664389df/crates/gdcef/src/accelerated_osr/mod.rs)
(lines 263–298). Platform implementations preserve an OS object, not ownership
of its frame contents:

- [Windows D3D12](https://github.com/dsh0416/godot-cef/blob/e78587f43863d4b454461148cbfc8f1e664389df/crates/gdcef/src/accelerated_osr/windows/d3d12.rs), lines 251–343: duplicate now, open/copy later.
- [Windows Vulkan](https://github.com/dsh0416/godot-cef/blob/e78587f43863d4b454461148cbfc8f1e664389df/crates/gdcef/src/accelerated_osr/windows/vulkan.rs), lines 278–400: defer the copy and cache imports by raw handle and dimensions. Handle reuse also makes this an unsafe resource identity.
- [macOS Metal](https://github.com/dsh0416/godot-cef/blob/e78587f43863d4b454461148cbfc8f1e664389df/crates/gdcef/src/accelerated_osr/macos.rs), lines 240–315: retain IOSurface, copy later. Waiting for the later Metal copy cannot restore the expired CEF lease.
- [Linux Vulkan](https://github.com/dsh0416/godot-cef/blob/e78587f43863d4b454461148cbfc8f1e664389df/crates/gdcef/src/accelerated_osr/linux/vulkan.rs), lines 399–563: duplicate/cache DMA-BUF resources and consume later; the same callback contract applies.

The pinned [Windows paint-info structure](https://github.com/chromiumembedded/cef/blob/708dc140cbc3286826a8abef89dc23a44ff9ea72/include/internal/cef_types_win.h)
explicitly provides no keyed mutex. Adding a mutex wait to these CEF handles is
not a synchronization protocol.

For this pinned Chromium path, producer completion is already addressed:
the mappable-shared-image copy result is delivered after the GPU-finished
callback in [Skia output](https://github.com/chromium/chromium/blob/152.0.7977.83/components/viz/service/display_embedder/skia_output_surface_impl_on_gpu.cc#L1063-L1106).
That permits consuming pixels during the callback; it gives no permission to
keep reading them after return.

## 2. Godot must participate in destination synchronization

There is currently one main destination RID. Windows Vulkan's two command
buffers/fences are not two independently owned presentation textures.

The D3D12 importer submits to a private queue and waits for its previous copy,
but does not order Godot's sampling against the new copy or protect against
Godot's previous reads. D3D11-on-12 wrapping and a private fence do not inform
Godot's resource-state tracker. Vulkan similarly issues native submissions
outside the render graph; a plugin-local mutex does not serialize Godot's own
submissions to the same queue.

Additional confirmed defects to remove with that path:

- [`vulkan_common.rs`](https://github.com/dsh0416/godot-cef/blob/e78587f43863d4b454461148cbfc8f1e664389df/crates/gdcef/src/accelerated_osr/vulkan_common.rs), lines 209–258: the Windows source transition uses `UNDEFINED`, which permits discarding source pixels. The destination transition also omits dependency on previous sampling. Full overwrite does not remove the need to order previous reads before the write. See [Vulkan image barriers](https://docs.vulkan.org/refpages/latest/refpages/source/VkImageMemoryBarrier.html).
- The same file, lines 130–135, chooses queue index 1 from physical-device capacity. [Godot 4.5 creates one queue per family](https://github.com/godotengine/godot/blob/4.5-stable/drivers/vulkan/rendering_device_driver_vulkan.cpp#L1044-L1065). Capacity does not mean that queue was requested at device creation; requesting a nonexistent queue violates [`vkGetDeviceQueue`](https://docs.vulkan.org/refpages/latest/refpages/source/vkGetDeviceQueue.html).
- [`windows/vulkan.rs`](https://github.com/dsh0416/godot-cef/blob/e78587f43863d4b454461148cbfc8f1e664389df/crates/gdcef/src/accelerated_osr/windows/vulkan.rs), lines 344–348, preserves a pending frame on fence timeout but returns `Ok(())`. [`AcceleratedRenderState::process_pending_copy`](https://github.com/dsh0416/godot-cef/blob/e78587f43863d4b454461148cbfc8f1e664389df/crates/gdcef/src/accelerated_osr/mod.rs), lines 170–178, then clears the outer pending flag, so the intended retry may never run without another paint.
- [`backend.rs`](https://github.com/dsh0416/godot-cef/blob/e78587f43863d4b454461148cbfc8f1e664389df/crates/gdcef/src/cef_texture/backend.rs), lines 403–419, frees the old RID before allocation/copy of its replacement succeeds. Teardown at lines 547–585 also frees destinations before closing the browser and before explicitly retiring the native importer work. Godot's deferred freeing cannot account for untracked external GPU work.

### Important Godot 4.5 API limitation

Moving native work to `RenderingServer.call_on_render_thread()` establishes CPU
thread ownership, not GPU dependencies or texture layouts. `frame_pre_draw`
and `frame_post_draw` are not GPU-completion fences.

[`RenderingDevice.texture_copy`](https://github.com/godotengine/godot/blob/4.5-stable/servers/rendering/rendering_device.cpp)
does register source and destination with Godot's graph, so it is a suitable
publication operation **after** a valid external-resource handoff. However,
`texture_create_from_extension()` alone does not supply that handoff: its
texture tracker starts with no established usage, mapped to an undefined layout
by the [render graph](https://github.com/godotengine/godot/blob/4.5-stable/servers/rendering/rendering_device_graph.cpp).
The public 4.5 API does not provide the full external acquire/release and
completion interface needed here; main-device `submit()`/`sync()` are not an
escape hatch, since they are for local devices.

The accelerated design needs a verified path to establish incoming resource
state, register the copy and its dependencies, and report completion for
resource retirement. Do not advertise a bare `texture_create_from_extension()`
+ `texture_copy()` rewrite as the complete fix.

A concrete engine-side route is to expose resource-aware native graph commands
(the existing C++ `RenderingDevice::driver_callback_add` is not public 4.5
GDExtension API), together with submission-completion reporting. The bridge
would declare destination copy usage, record the owned-snapshot copy in Godot's
command stream, handle source acquisition/state, and return a retirement token.
This is one possible integration route, not an API already available to this crate.

### Later versions and alternatives

The API/source audit also covers Godot 4.6.3, 4.7.2 and master as of the
investigation date. None exposes a complete external-resource acquire/release
and submission-completion contract through GDExtension. The native
`driver_callback_add` method remains unbound. Upgrading alone does not solve
the handoff. See the source links and version details in #227.

There is a possible public-API retirement mechanism: enqueue a tracked copy
from the displayed texture to a per-slot 1×1 sentinel, then request
`texture_get_data_async` on the sentinel. The readback callback runs after the
frame fence. This may conservatively retire preceding tracked use with only
a small readback, at the cost of latency and staging overhead. It does not
establish the initial state of an imported source.

A backend-specific API-only path may be possible for Metal, or D3D12 when the
owned snapshot's native state exactly matches the imported tracker's state.
Vulkan's undefined initial layout remains an obstacle. These are proposals
requiring validation, not working guarantees.

An alternative to an engine patch is a narrow native copy hook: queue a normal
tracked `texture_copy` from a dedicated marker RID to the destination and
replace only that marker's native copy source with a completed owned snapshot.
Godot would retain destination dependency tracking; the adapter would handle
the real source's acquisition and lifetime. This needs per-backend validation,
strict handle/generation matching and fail-closed installation. No such hook
is implemented or approved by this investigation.

## 3. Drag and focus need separate lifecycle fixes

### Drag

[`handle_start_dragging`](https://github.com/dsh0416/godot-cef/blob/e78587f43863d4b454461148cbfc8f1e664389df/crates/gdcef/src/webrender.rs), lines 248–266,
always returns true, even for missing drag data or failed queue insertion.
[`emit_drag_event_signals`](https://github.com/dsh0416/godot-cef/blob/e78587f43863d4b454461148cbfc8f1e664389df/crates/gdcef/src/cef_texture/signals.rs), lines
277–299, merely marks a boolean and emits an optional signal. Completion depends
entirely on the user calling methods in
[`cef_texture/mod.rs`](https://github.com/dsh0416/godot-cef/blob/e78587f43863d4b454461148cbfc8f1e664389df/crates/gdcef/src/cef_texture/mod.rs), lines 771–805.
Standalone `CefTexture2D` even
[discards the queued drag event](https://github.com/dsh0416/godot-cef/blob/e78587f43863d4b454461148cbfc8f1e664389df/crates/gdcef/src/cef_texture2d/runtime.rs)
(lines 140–153).

[PR #192](https://github.com/dsh0416/godot-cef/pull/192) paired and guarded the
explicit completion calls. It did not add a default owner or cancellation path,
which explains why it could not cover a minimal scene without a drag handler.
Selecting already-selected text can initiate a source drag; this is a plausible
mechanism for #207, to verify with a callback trace.

Fix the acceptance contract first. Without a registered adapter capable of
owning the operation, return false. With an adapter, establish a session before
returning true and terminate it exactly once on success or cancellation.
Use browser-generation/session IDs so a delayed completion cannot end a newer
drag. Handle Escape, capture loss, adapter failure and teardown; define whether
window focus loss cancels or is handled by an active OS drag adapter. A signal
listener's existence alone is not a completion guarantee. Follow the pinned
[StartDragging contract](https://github.com/chromiumembedded/cef/blob/708dc140cbc3286826a8abef89dc23a44ff9ea72/include/cef_render_handler.h)
and [BrowserHost completion/cancellation methods](https://github.com/chromiumembedded/cef/blob/708dc140cbc3286826a8abef89dc23a44ff9ea72/include/cef_browser.h).

### Focus and IME

[`CefTexture` focus notifications](https://github.com/dsh0416/godot-cef/blob/e78587f43863d4b454461148cbfc8f1e664389df/crates/gdcef/src/cef_texture/mod.rs),
lines 144–152, directly set CEF focus. In
[`ime.rs`](https://github.com/dsh0416/godot-cef/blob/e78587f43863d4b454461148cbfc8f1e664389df/crates/gdcef/src/cef_texture/ime.rs), first activation transfers
focus to the hidden LineEdit and restores CEF focus (115–131). The deferred
repeated-click path performs the same transfer without restoring it (90–107).
Deactivation then unconditionally grabs CefTexture focus (135–148), even when
the user moved to an unrelated control.

Treat CefTexture and its proxy as one logical browser focus owner. Keep window
focus, logical browser focus, DOM editability, and composition state separate.
Reconcile internal transfers without sending CEF a blur; external focus loss
must relinquish ownership. DOM editability messages may enable IME only for the
current owner and browser/document generation, not reclaim focus.

Input routing belongs to that owner too. Currently `_input()` forwards global
events without bounds/focus/capture checks, and converting positions mutates
the shared Godot event object (`event.clone()` clones its handle). Copy values
or duplicate the event, route pointer input by hit testing/capture, send keys
only to the logical owner, and deliver mouse-leave/capture-loss transitions.

## 4. Recommended frame ownership protocol

```text
CEF UI callback                         Godot render integration
borrowed CEF frame
    -> complete capture to owned slot
    -> publish newest Ready slot  --->  acquire owned slot
return borrowed CEF frame               -> tracked copy into display texture
                                       -> retire slot after GPU read completes
                                       -> sample display texture in graph order
```

Each slot follows `Free -> Capturing -> Ready -> Reading -> Retiring -> Free`.
`Reading/Retiring` means Godot's GPU may still use it, not merely that a CPU
function is running. Slot metadata should include browser generation, capture
sequence, surface kind (view/popup), dimensions/format, and completion tokens.

1. **Capture inside the callback.** Reopen CEF's resource every time. Complete
   the source-reading copy before callback return. Windows can use D3D11 to
   capture into owned shareable textures, Metal an owned texture, and Linux a
   validated DMA-BUF import/copy path with required ownership transitions.
   Keep these capture resources separate from Godot's live display RID.
2. **Bound storage and admit before submission.** Start with a small pool, for
   example three slots, and drop a new frame before copying if every slot is in
   use. Replace obsolete completed `Ready` frames, never a slot still used by a
   GPU. Present the newest completed sequence and retain the last good display
   on temporary failure. Pool size is a throughput choice, not synchronization.
3. **Publish through the render graph/bridge.** Establish the owned source's
   real state and readiness; perform a tracked copy into the stable display
   RID. Recycle the staging source only after the copy finishes reading it.
   Directly displaying/swapping owned slots is possible only if their final
   sampling completion is tracked instead. Both directions need synchronization.
4. **Handle resize and popup generations explicitly.** Allocate and fill a new
   display texture before rebinding, then retire the old one after tracked use.
   Give view and popup independent pending slots; current popup work can consume
   the same importer's pending slot. Reject stale browser/popup generations and
   incompatible dimensions. Keep the existing display while a new size is
   unavailable. Begin with full-frame copies: skipping frames makes unaccumulated
   dirty-rectangle updates unsafe.
5. **Close in ownership order.** Stop accepting input/capture, cancel owned
   interactions, invalidate generations, close the browser while the CEF pump
   remains alive, and retire GPU work before destroying its resources. Drain
   metadata under short locks and release them before Godot/CEF calls or waits.

### Completion and failure semantics

Submission is not completion. Microsoft documents
[`CopyResource`](https://learn.microsoft.com/en-us/windows/win32/api/d3d11/nf-d3d11-id3d11devicecontext-copyresource)
and [`Flush`](https://learn.microsoft.com/en-us/windows/win32/api/d3d11/nf-d3d11-id3d11devicecontext-flush)
as asynchronous. A real fence/query/command-buffer completion must establish
that the CEF source is no longer being read.

Once a source-reading copy is submitted, a timeout cannot safely return from
the CEF callback: that would release storage still in use. Drop before submit;
after submit, complete it or use a documented device-loss/teardown path that
proves outstanding reads cannot execute. Do not turn a timeout into `Ready`,
clear retry state on `Pending`, or promise both nonblocking callbacks and safe
borrowed-source reuse with the current API.

For fully asynchronous capture, CEF needs an upstream lease/release API that
defers the underlying `Done()` until consumer completion, or a supported release
fence. That is an optimization beyond the synchronous snapshot baseline.
Godot still needs a verified acquire/state/retirement path. Arbitrary extra
buffering or input acknowledgments cannot replace either contract.

## 5. One process-wide pump, with ordered input

[`cef_init.rs`](https://github.com/dsh0416/godot-cef/blob/e78587f43863d4b454461148cbfc8f1e664389df/crates/gdcef/src/cef_init.rs), line 257, enables external
message pumping, but [`browser_process.rs`](https://github.com/dsh0416/godot-cef/blob/e78587f43863d4b454461148cbfc8f1e664389df/crates/cef_app/src/browser_process.rs)
does not implement `OnScheduleMessagePumpWork`. Each CefTexture `_process` or
standalone CefTexture2D tick independently pumps the process. Both upload before
pumping CEF, so a paint delivered by that pump waits until a later update.
The helper owned by CefTexture disconnects its own hook; the duplication is
across live browser instances, not two active hooks for every CefTexture.

Use one runtime service on CEF's required UI/main thread. Implement deadline
updates and main-thread wakeups per
[OnScheduleMessagePumpWork](https://github.com/chromiumembedded/cef/blob/708dc140cbc3286826a8abef89dc23a44ff9ea72/include/cef_browser_process_handler.h).
Keep it alive for browser creation/closure and pending tasks when drawing is
paused or no texture is visible. A per-frame Godot Timer alone cannot guarantee
responsive CEF work while the main loop is throttled; the wakeup integration
must be tested on each platform. A supported CEF multi-threaded loop is a
separate option requiring a full thread-affinity audit, not a setting-only fix.

Deliver input/focus commands in order; coalesce only compatible pointer-motion
updates, never across button, key, composition or capture boundaries. Request
external begin frames at the chosen cadence and consume the newest completed
paint before presentation. Never spin until a paint arrives: unchanged content
may produce no paint, and `SendExternalBeginFrame` is not a presentation barrier.

Diagnostic input sequence numbers describe submission/processing order. They
do not prove that a particular accelerated paint contains a particular input;
CEF provides no such correlation token. A test page can render its own input
counter for measurement. The target is ordered, coherent output with bounded
buffering, not an unsupported promise of zero-frame latency.

## 6. Delivery and validation

Recommended implementation sequence:

1. Fix default drag rejection/session ownership and logical focus/input routing.
   These should also pass with accelerated OSR disabled; they are independently
   testable and should not wait for renderer integration work.
2. Centralize the CEF scheduler. Add frame capture/presentation and interaction
   tracing sufficient to distinguish input stalls, pump stalls and GPU stalls.
3. Establish the owned-snapshot protocol and a tested Godot GPU bridge, starting
   with Windows D3D12 and Vulkan. Port the same invariants to Metal/Linux.
4. Gate accelerated support on the complete handoff capability. The existing
   software `OnPaint` path copies bytes within the callback and uploads through
   Godot; it is the available correctness baseline for the frame handoff where
   a GPU bridge is unavailable. It does not fix drag/focus defects by itself.
5. Optimize capture waits, import reuse of **owned** textures, dirty regions,
   and optional upstream CEF release fences only after correctness validation.

Required regression evidence:

| Area | Exercise | Acceptance evidence |
| --- | --- | --- |
| Borrowed source | Deterministic source fixture overwrites/recycles the resource immediately after callback return | Presented snapshot remains the captured frame; import caches never identify a CEF frame by old handle |
| Real frames | Local page with visible monotonic frame/input counters, alternating colors, heavy images and scroll | No counter rollback or mixed frame; repeated frames/dropped intermediate frames are allowed |
| GPU ownership | Delayed capture/presentation, slow consumer, slot exhaustion, device errors | No write to a live slot; correct pending/retry states; validation layers report no resource/queue hazards |
| Drag | Repeated select/drag with zero signal handlers; accepted adapter sessions; Escape/capture loss/disposal | Unsupported requests reject; every accepted session terminates once; stale completion cannot affect the next |
| Focus/IME | Browser/body/input/native LineEdit switching, Chinese/Japanese composition, Alt-Tab, two browser controls | One logical owner; no focus stealing, dropped composition or duplicate input |
| Lifetime | Continuous resize, popup show/hide/resize, navigation, create/destroy and close with copies pending | Old generations never publish; every source/destination is retired before destruction |
| Scheduling | 30/60/144 Hz, uncapped, low-FPS/minimized/paused scene, multiple browsers | Pump not multiplied by browser count; deadlines serviced; input/pump/presentation latency measured separately |

Run Windows 11 D3D12, Vulkan and software paths first, on Godot 4.5 and the
reporters' 4.6 line, with D3D/Vulkan validation enabled. Then validate macOS
Metal and Linux Vulkan. Record an independent page heartbeat alongside drag,
focus, pump and GPU-completion traces so a stuck interaction is not mislabeled
as a frozen renderer.

Investigation validation: current issue bodies/comments, PR #192, local call
paths, pinned CEF generated headers and upstream CEF/Chromium/Godot source were
inspected. No Windows execution or performance measurements were performed.
The machine's default local CEF SDK is 145.0.22, so it must not be mistaken for
the pinned 152.0.6 runtime when doing subsequent builds or reproduction.
