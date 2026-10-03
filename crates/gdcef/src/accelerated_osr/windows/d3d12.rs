use super::super::{NativeCaptureTarget, SnapshotFormat};
use godot::classes::RenderingServer;
use godot::classes::rendering_device::DriverResource;
use godot::global::{godot_print, godot_warn};
use godot::prelude::*;
use std::ffi::{CStr, c_void};
use std::io::Write;
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use windows::Win32::Foundation::{CloseHandle, HANDLE, LUID, WAIT_FAILED};
use windows::Win32::Graphics::Direct3D12::{
    D3D12_BARRIER_ACCESS_COMMON, D3D12_BARRIER_ACCESS_COPY_SOURCE, D3D12_BARRIER_GROUP,
    D3D12_BARRIER_GROUP_0, D3D12_BARRIER_LAYOUT_COMMON, D3D12_BARRIER_LAYOUT_COPY_SOURCE,
    D3D12_BARRIER_SUBRESOURCE_RANGE, D3D12_BARRIER_SYNC_ALL, D3D12_BARRIER_TYPE_TEXTURE,
    D3D12_COMMAND_LIST_TYPE_DIRECT, D3D12_COMMAND_QUEUE_DESC, D3D12_FEATURE_D3D12_OPTIONS12,
    D3D12_FEATURE_DATA_D3D12_OPTIONS12, D3D12_FENCE_FLAG_NONE, D3D12_MESSAGE,
    D3D12_MESSAGE_CALLBACK_IGNORE_FILTERS, D3D12_MESSAGE_CATEGORY, D3D12_MESSAGE_ID,
    D3D12_MESSAGE_SEVERITY, D3D12_MESSAGE_SEVERITY_CORRUPTION, D3D12_MESSAGE_SEVERITY_ERROR,
    D3D12_RESOURCE_BARRIER, D3D12_RESOURCE_BARRIER_0, D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
    D3D12_RESOURCE_BARRIER_TYPE_TRANSITION, D3D12_RESOURCE_DIMENSION_TEXTURE2D,
    D3D12_RESOURCE_FLAG_ALLOW_SIMULTANEOUS_ACCESS, D3D12_RESOURCE_STATE_COMMON,
    D3D12_RESOURCE_STATE_COPY_DEST, D3D12_RESOURCE_STATE_COPY_SOURCE, D3D12_RESOURCE_STATES,
    D3D12_RESOURCE_TRANSITION_BARRIER, D3D12_TEXTURE_BARRIER, ID3D12CommandAllocator,
    ID3D12CommandList, ID3D12CommandQueue, ID3D12Device, ID3D12Fence, ID3D12GraphicsCommandList,
    ID3D12GraphicsCommandList7, ID3D12InfoQueue, ID3D12InfoQueue1, ID3D12Resource,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_TYPELESS, DXGI_FORMAT_B8G8R8A8_UNORM,
    DXGI_FORMAT_B8G8R8A8_UNORM_SRGB, DXGI_FORMAT_R8G8B8A8_TYPELESS, DXGI_FORMAT_R8G8B8A8_UNORM,
    DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory, IDXGIAdapter, IDXGIFactory};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::core::{IUnknown, Interface, PCSTR};

/// Captures the CEF borrow synchronously into an exclusively owned staging slot.
/// Native context and fence access is serialized by the containing importer
/// mutex. No borrowed source handle survives `capture`.
pub struct D3D12TextureImporter {
    device: ID3D12Device,
    copy: NativeCopyList,
    capture_queue: ID3D12CommandQueue,
    godot_queue: ID3D12CommandQueue,
    capture_fence: ID3D12Fence,
    release_fence: ID3D12Fence,
    fence_value: u64,
    fence_event: FenceEvent,
    enhanced_bridge: Option<EnhancedBridge>,
    diagnostics: Option<NativeDebugDiagnostics>,
}

impl D3D12TextureImporter {
    /// Must run on Godot's render thread; capture itself uses only native APIs.
    pub fn new() -> Result<Self, String> {
        let rd = RenderingServer::singleton()
            .get_rendering_device()
            .ok_or("Failed to get RenderingDevice")?;
        let device_ptr = rd.get_driver_resource(DriverResource::LOGICAL_DEVICE, Rid::Invalid, 0);
        let queue_ptr = rd.get_driver_resource(DriverResource::COMMAND_QUEUE, Rid::Invalid, 0);
        // Godot retains its references; clone adds our own COM references.
        let device: ID3D12Device = unsafe { clone_native_interface(device_ptr) }?;
        let diagnostics = NativeDebugDiagnostics::new(&device);
        let godot_queue = godot_command_queue(queue_ptr, &device)?;
        if unsafe { godot_queue.GetDesc() }.Type != D3D12_COMMAND_LIST_TYPE_DIRECT {
            return Err("Godot's main D3D12 queue is not a direct queue".into());
        }
        let capture_queue: ID3D12CommandQueue = unsafe {
            device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
                Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
                ..Default::default()
            })
        }
        .map_err(|e| format!("CreateCommandQueue failed: {e}"))?;
        let capture_fence = unsafe { device.CreateFence(0, D3D12_FENCE_FLAG_NONE) }
            .map_err(|e| format!("Create capture fence failed: {e}"))?;
        let release_fence = unsafe { device.CreateFence(0, D3D12_FENCE_FLAG_NONE) }
            .map_err(|e| format!("Create release fence failed: {e}"))?;
        let fence_event = FenceEvent(
            unsafe { CreateEventW(None, false, false, None) }
                .map_err(|e| format!("Create fence event failed: {e}"))?,
        );

        // This matches Godot 4.6's D3D12 feature selection. Enhanced and legacy
        // COPY_SOURCE are NOT interchangeable: interop requires a COMMON bridge.
        // https://github.com/godotengine/godot/blob/89cea143987d564363e15d207438530651d943ac/drivers/d3d12/rendering_device_driver_d3d12.cpp#L5574-L5578
        // https://microsoft.github.io/DirectX-Specs/d3d/D3D12EnhancedBarriers.html#interop-with-legacy-resourcebarrier
        let mut options = D3D12_FEATURE_DATA_D3D12_OPTIONS12::default();
        let enhanced = unsafe {
            device.CheckFeatureSupport(
                D3D12_FEATURE_D3D12_OPTIONS12,
                &mut options as *mut _ as *mut c_void,
                size_of::<D3D12_FEATURE_DATA_D3D12_OPTIONS12>() as u32,
            )
        }
        .is_ok()
            && options.EnhancedBarriersSupported.as_bool();
        let enhanced_bridge = if enhanced {
            Some(EnhancedBridge::new(&device)?)
        } else {
            None
        };

        let copy = NativeCopyList::new(&device)?;
        godot_print!(
            "[AcceleratedOSR/D3D12] Synchronous staging capture (enhanced barriers: {})",
            enhanced
        );
        Ok(Self {
            device,
            copy,
            capture_queue,
            godot_queue,
            capture_fence,
            release_fence,
            fence_value: 0,
            fence_event,
            enhanced_bridge,
            diagnostics,
        })
    }

    pub fn capture(
        &mut self,
        info: &cef::AcceleratedPaintInfo,
        target: NativeCaptureTarget,
    ) -> Result<(), String> {
        self.check_device_state()?;
        let handle = HANDLE(info.shared_texture_handle);
        if handle.is_invalid() {
            return Err("CEF supplied an invalid shared texture handle".into());
        }
        if info.extra.coded_size.width <= 0
            || info.extra.coded_size.height <= 0
            || target.width != info.extra.coded_size.width as u32
            || target.height != info.extra.coded_size.height as u32
        {
            return Err("CEF dimensions do not match the staging slot".into());
        }
        let expected_color = match target.format {
            SnapshotFormat::Bgra8 => cef::ColorType::BGRA_8888,
            SnapshotFormat::Rgba8 => cef::ColorType::RGBA_8888,
        };
        if info.format != expected_color {
            return Err("CEF pixel format does not match the staging slot".into());
        }
        // Duplicating a HANDLE would extend handle lifetime, not preserve pixels.
        let mut source: Option<ID3D12Resource> = None;
        unsafe { self.device.OpenSharedHandle(handle, &mut source) }
            .map_err(|e| format!("OpenSharedHandle failed: {e}"))?;
        let source = source.ok_or("CEF shared handle resolved to a null resource")?;
        let destination: ID3D12Resource = unsafe { clone_native_interface(target.native_handle) }?;
        validate_copy_resources(&source, &destination, target)?;
        let next_value = self
            .fence_value
            .checked_add(1)
            .filter(|value| *value != u64::MAX)
            .ok_or("D3D12 capture fence timeline exhausted")?;

        // Finish all fallible recording before submitting anything.
        if let Some(bridge) = self.enhanced_bridge.as_mut() {
            bridge.record(&destination)?;
        }
        let state = if self.enhanced_bridge.is_some() {
            D3D12_RESOURCE_STATE_COMMON
        } else {
            // RD explicitly transitions initialized staging from COPY_DEST to
            // COPY_SOURCE: this is not an implicitly promoted/decaying state.
            D3D12_RESOURCE_STATE_COPY_SOURCE
        };
        self.copy.record(&source, &destination, state)?;

        // RD retirement has already completed, but still express queue-to-queue
        // ownership and visibility before native writes begin.
        unsafe { self.godot_queue.Signal(&self.release_fence, next_value) }
            .map_err(|e| format!("Signal staging release failed: {e}"))?;
        unsafe { self.capture_queue.Wait(&self.release_fence, next_value) }
            .map_err(|e| format!("Wait for staging release failed: {e}"))?;
        self.fence_value = next_value;
        if let Some(bridge) = &self.enhanced_bridge {
            bridge.acquire.submit(&self.capture_queue);
        }
        self.copy.submit(&self.capture_queue);
        if let Some(bridge) = &self.enhanced_bridge {
            bridge.release.submit(&self.capture_queue);
        }
        // Retain source until GPU completion or actual device
        // removal. An ordinary Signal/SetEvent failure cannot end CEF's borrow.
        let result = self.finish_submitted_capture();
        if let Some(diagnostics) = &self.diagnostics {
            diagnostics.poll();
        }
        result
    }

    /// Run on the render thread before recording RD staging -> display copies.
    /// CPU completion protects the CEF borrow; the queue wait expresses producer
    /// visibility to D3D12 and its validation layer.
    pub fn prepare_publication(&self) -> Result<(), String> {
        if let Some(diagnostics) = &self.diagnostics {
            diagnostics.poll();
        }
        self.check_device_state()?;
        if self.fence_value == 0 {
            return Err("No completed D3D12 snapshot to publish".into());
        }
        unsafe { self.godot_queue.Wait(&self.capture_fence, self.fence_value) }
            .map_err(|e| format!("Wait for captured snapshot failed: {e}"))
    }

    fn check_device_state(&self) -> Result<(), String> {
        unsafe { self.device.GetDeviceRemovedReason() }
            .map_err(|e| format!("D3D12 device removed: {e}"))
    }

    fn finish_submitted_capture(&self) -> Result<(), String> {
        let mut signal_error_logged = false;
        loop {
            match unsafe {
                self.capture_queue
                    .Signal(&self.capture_fence, self.fence_value)
            } {
                Ok(()) => break,
                Err(error) => {
                    self.check_device_state()?;
                    if !signal_error_logged {
                        godot_warn!(
                            "[AcceleratedOSR/D3D12] Fence signal failed ({error}); retaining CEF's borrow until completion or device removal"
                        );
                        signal_error_logged = true;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
        let mut use_event = unsafe {
            self.capture_fence
                .SetEventOnCompletion(self.fence_value, self.fence_event.0)
        }
        .is_ok();
        loop {
            let completed = unsafe { self.capture_fence.GetCompletedValue() };
            // UINT64_MAX signals removal; it is never a successful frame.
            if completed != u64::MAX && completed >= self.fence_value {
                return self.check_device_state();
            }
            self.check_device_state()?;
            if use_event {
                if unsafe { WaitForSingleObject(self.fence_event.0, 100) } == WAIT_FAILED {
                    use_event = false;
                }
            } else {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
}

fn validate_copy_resources(
    source: &ID3D12Resource,
    destination: &ID3D12Resource,
    target: NativeCaptureTarget,
) -> Result<(), String> {
    let source_desc = unsafe { source.GetDesc() };
    let destination_desc = unsafe { destination.GetDesc() };
    let same_family = |format: DXGI_FORMAT| match target.format {
        SnapshotFormat::Bgra8 => matches!(
            format,
            DXGI_FORMAT_B8G8R8A8_UNORM
                | DXGI_FORMAT_B8G8R8A8_UNORM_SRGB
                | DXGI_FORMAT_B8G8R8A8_TYPELESS
        ),
        SnapshotFormat::Rgba8 => matches!(
            format,
            DXGI_FORMAT_R8G8B8A8_UNORM
                | DXGI_FORMAT_R8G8B8A8_UNORM_SRGB
                | DXGI_FORMAT_R8G8B8A8_TYPELESS
        ),
    };
    if source_desc.Dimension != D3D12_RESOURCE_DIMENSION_TEXTURE2D
        || source_desc.Width != u64::from(target.width)
        || source_desc.Height != target.height
        || source_desc.MipLevels != 1
        || source_desc.DepthOrArraySize != 1
        || source_desc.SampleDesc.Count != 1
        || !same_family(source_desc.Format)
        || destination_desc.Dimension != D3D12_RESOURCE_DIMENSION_TEXTURE2D
        || destination_desc.Width != u64::from(target.width)
        || destination_desc.Height != target.height
        || destination_desc.MipLevels != 1
        || destination_desc.DepthOrArraySize != 1
        || destination_desc.SampleDesc.Count != 1
        || !same_family(destination_desc.Format)
        || destination_desc
            .Flags
            .contains(D3D12_RESOURCE_FLAG_ALLOW_SIMULTANEOUS_ACCESS)
    {
        return Err("Capture requires matching single-subresource RGBA8/BGRA8 textures without simultaneous access".into());
    }
    Ok(())
}

struct NativeCopyList {
    allocator: ID3D12CommandAllocator,
    list: ID3D12GraphicsCommandList,
    submission: ID3D12CommandList,
}

impl NativeCopyList {
    fn new(device: &ID3D12Device) -> Result<Self, String> {
        let allocator = unsafe { device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT) }
            .map_err(|e| format!("Create copy allocator failed: {e}"))?;
        let list: ID3D12GraphicsCommandList = unsafe {
            device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &allocator, None)
        }
        .map_err(|e| format!("Create copy command list failed: {e}"))?;
        let submission = list
            .cast()
            .map_err(|e| format!("Query copy command list failed: {e}"))?;
        unsafe { list.Close() }.map_err(|e| format!("Close initial copy list failed: {e}"))?;
        Ok(Self {
            allocator,
            list,
            submission,
        })
    }

    fn record(
        &mut self,
        source: &ID3D12Resource,
        destination: &ID3D12Resource,
        destination_state: D3D12_RESOURCE_STATES,
    ) -> Result<(), String> {
        // Previous capture completion protects both allocator and list reuse.
        unsafe { self.allocator.Reset() }
            .map_err(|e| format!("Reset copy allocator failed: {e}"))?;
        unsafe { self.list.Reset(&self.allocator, None) }
            .map_err(|e| format!("Reset copy list failed: {e}"))?;
        // This COMMON contract applies to CEF's shared D3D11 source, not Godot's
        // staging texture. Pinned CEF 154.0.28 requests mappable shared frames;
        // its Chromium waits for GPU completion before delivering that borrow:
        // https://github.com/chromiumembedded/cef/blob/564dd6c4aafff558154bd3176eb5d13551db6734/libcef/browser/osr/video_consumer_osr.cc#L43-L49
        // https://github.com/chromium/chromium/blob/a654841425914cbb703a2931e07b70a83aedbafd/components/viz/service/display_embedder/skia_output_surface_impl_on_gpu.cc#L1063-L1129
        // Chromium creates the Windows GMB as a shared D3D11 texture. Microsoft's
        // D3D11On12 shared-resource import starts in COMMON; restore COMMON before
        // returning the source. The staging state uses its separate RD contract.
        // https://github.com/chromium/chromium/blob/a654841425914cbb703a2931e07b70a83aedbafd/gpu/command_buffer/service/shared_image/d3d_image_backing_factory.cc#L380-L410
        // https://github.com/microsoft/D3D11On12/blob/ed0213477e4ed9d6f929122586f6ec348793af16/src/resource.cpp#L349-L354
        self.transition(
            source,
            D3D12_RESOURCE_STATE_COMMON,
            D3D12_RESOURCE_STATE_COPY_SOURCE,
        );
        self.transition(
            destination,
            destination_state,
            D3D12_RESOURCE_STATE_COPY_DEST,
        );
        unsafe { self.list.CopyResource(destination, source) };
        self.transition(
            source,
            D3D12_RESOURCE_STATE_COPY_SOURCE,
            D3D12_RESOURCE_STATE_COMMON,
        );
        self.transition(
            destination,
            D3D12_RESOURCE_STATE_COPY_DEST,
            destination_state,
        );
        unsafe { self.list.Close() }.map_err(|e| format!("Close copy list failed: {e}"))
    }

    fn transition(
        &self,
        resource: &ID3D12Resource,
        before: D3D12_RESOURCE_STATES,
        after: D3D12_RESOURCE_STATES,
    ) {
        let mut barrier = D3D12_RESOURCE_BARRIER {
            Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            Anonymous: D3D12_RESOURCE_BARRIER_0 {
                Transition: ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                    pResource: ManuallyDrop::new(Some(resource.clone())),
                    Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                    StateBefore: before,
                    StateAfter: after,
                }),
            },
            ..Default::default()
        };
        unsafe {
            self.list.ResourceBarrier(std::slice::from_ref(&barrier));
            ManuallyDrop::drop(&mut (*barrier.Anonymous.Transition).pResource);
        }
    }

    fn submit(&self, queue: &ID3D12CommandQueue) {
        unsafe { queue.ExecuteCommandLists(&[Some(self.submission.clone())]) };
    }
}

struct EnhancedBridge {
    acquire: BarrierList,
    release: BarrierList,
}

impl EnhancedBridge {
    fn new(device: &ID3D12Device) -> Result<Self, String> {
        Ok(Self {
            acquire: BarrierList::new(device)?,
            release: BarrierList::new(device)?,
        })
    }

    fn record(&mut self, resource: &ID3D12Resource) -> Result<(), String> {
        self.acquire.record(resource, true)?;
        self.release.record(resource, false)
    }
}

struct BarrierList {
    allocator: ID3D12CommandAllocator,
    list: ID3D12GraphicsCommandList7,
    submission: ID3D12CommandList,
}

impl BarrierList {
    fn new(device: &ID3D12Device) -> Result<Self, String> {
        let allocator = unsafe { device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT) }
            .map_err(|e| format!("Create barrier allocator failed: {e}"))?;
        let list: ID3D12GraphicsCommandList7 = unsafe {
            device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &allocator, None)
        }
        .map_err(|e| format!("Create enhanced barrier list failed: {e}"))?;
        let submission = list
            .cast()
            .map_err(|e| format!("Query barrier command list failed: {e}"))?;
        unsafe { list.Close() }.map_err(|e| format!("Close initial barrier list failed: {e}"))?;
        Ok(Self {
            allocator,
            list,
            submission,
        })
    }

    fn record(&mut self, resource: &ID3D12Resource, acquire: bool) -> Result<(), String> {
        // Every previous capture completed before either allocator can be reset.
        // Both lists are recorded before any borrowed-source copy is submitted.
        unsafe { self.allocator.Reset() }
            .map_err(|e| format!("Reset barrier allocator failed: {e}"))?;
        unsafe { self.list.Reset(&self.allocator, None) }
            .map_err(|e| format!("Reset barrier list failed: {e}"))?;
        let mut barrier = D3D12_TEXTURE_BARRIER {
            SyncBefore: D3D12_BARRIER_SYNC_ALL,
            SyncAfter: D3D12_BARRIER_SYNC_ALL,
            AccessBefore: if acquire {
                D3D12_BARRIER_ACCESS_COPY_SOURCE
            } else {
                D3D12_BARRIER_ACCESS_COMMON
            },
            AccessAfter: if acquire {
                D3D12_BARRIER_ACCESS_COMMON
            } else {
                D3D12_BARRIER_ACCESS_COPY_SOURCE
            },
            LayoutBefore: if acquire {
                D3D12_BARRIER_LAYOUT_COPY_SOURCE
            } else {
                D3D12_BARRIER_LAYOUT_COMMON
            },
            LayoutAfter: if acquire {
                D3D12_BARRIER_LAYOUT_COMMON
            } else {
                D3D12_BARRIER_LAYOUT_COPY_SOURCE
            },
            pResource: ManuallyDrop::new(Some(resource.clone())),
            Subresources: D3D12_BARRIER_SUBRESOURCE_RANGE {
                IndexOrFirstMipLevel: u32::MAX,
                ..Default::default()
            },
            ..Default::default()
        };
        unsafe {
            self.list.Barrier(&[D3D12_BARRIER_GROUP {
                Type: D3D12_BARRIER_TYPE_TEXTURE,
                NumBarriers: 1,
                Anonymous: D3D12_BARRIER_GROUP_0 {
                    pTextureBarriers: &barrier,
                },
            }]);
            ManuallyDrop::drop(&mut barrier.pResource);
        }
        unsafe { self.list.Close() }.map_err(|e| format!("Close barrier list failed: {e}"))
    }

    fn submit(&self, queue: &ID3D12CommandQueue) {
        unsafe { queue.ExecuteCommandLists(&[Some(self.submission.clone())]) };
    }
}

struct FenceEvent(HANDLE);

impl Drop for FenceEvent {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// Only exists when Godot actually created a debug-layer D3D12 device. No layer
/// or filters are enabled/changed here. Capture errors must be visible even if
/// the engine binary sends its own debug output only to a debugger.
struct NativeDebugDiagnostics {
    queue: ID3D12InfoQueue,
    callback: Option<(ID3D12InfoQueue1, u32)>,
    next_message: AtomicU64,
    discarded_messages: AtomicU64,
}

impl NativeDebugDiagnostics {
    fn new(device: &ID3D12Device) -> Option<Self> {
        let queue = device.cast::<ID3D12InfoQueue>().ok()?;
        let callback = queue.cast::<ID3D12InfoQueue1>().ok().and_then(|queue| {
            let mut cookie = 0;
            unsafe {
                queue.RegisterMessageCallback(
                    Some(native_debug_message),
                    D3D12_MESSAGE_CALLBACK_IGNORE_FILTERS,
                    std::ptr::null_mut(),
                    &mut cookie,
                )
            }
            .ok()
            .map(|()| (queue, cookie))
        });
        godot_print!(
            "[AcceleratedOSR/D3D12] Debug layer confirmed: ID3D12InfoQueue available; diagnostics {}",
            if callback.is_some() {
                "callback"
            } else {
                "polling"
            }
        );
        Some(Self {
            queue,
            callback,
            next_message: AtomicU64::new(0),
            discarded_messages: AtomicU64::new(0),
        })
    }

    fn poll(&self) {
        if self.callback.is_some() {
            return;
        }
        let count = unsafe { self.queue.GetNumStoredMessagesAllowedByRetrievalFilter() };
        let start = self.next_message.swap(count, Ordering::Relaxed);
        for index in (if start <= count { start } else { 0 })..count {
            let mut length = 0;
            if unsafe { self.queue.GetMessage(index, None, &mut length) }.is_err()
                || length < size_of::<D3D12_MESSAGE>()
            {
                continue;
            }
            // usize storage provides the alignment required by D3D12_MESSAGE.
            let mut storage = vec![0usize; length.div_ceil(size_of::<usize>())];
            let message = storage.as_mut_ptr().cast::<D3D12_MESSAGE>();
            if unsafe { self.queue.GetMessage(index, Some(message), &mut length) }.is_ok() {
                let message = unsafe { &*message };
                unsafe {
                    native_debug_message(
                        message.Category,
                        message.Severity,
                        message.ID,
                        PCSTR(message.pDescription),
                        std::ptr::null_mut(),
                    );
                }
            }
        }
        let discarded = unsafe { self.queue.GetNumMessagesDiscardedByMessageCountLimit() };
        if discarded > self.discarded_messages.swap(discarded, Ordering::Relaxed) {
            let _ = writeln!(
                std::io::stderr(),
                "D3D12 ERROR: debug message queue overflowed; diagnostics are incomplete"
            );
        }
    }
}

impl Drop for NativeDebugDiagnostics {
    fn drop(&mut self) {
        self.poll();
        if let Some((queue, cookie)) = &self.callback {
            let _ = unsafe { queue.UnregisterMessageCallback(*cookie) };
        }
    }
}

unsafe extern "system" fn native_debug_message(
    _category: D3D12_MESSAGE_CATEGORY,
    severity: D3D12_MESSAGE_SEVERITY,
    id: D3D12_MESSAGE_ID,
    description: PCSTR,
    _context: *mut c_void,
) {
    if !matches!(
        severity,
        D3D12_MESSAGE_SEVERITY_ERROR | D3D12_MESSAGE_SEVERITY_CORRUPTION
    ) || description.is_null()
    {
        return;
    }
    let message = unsafe { CStr::from_ptr(description.as_ptr().cast()) }.to_string_lossy();
    // Do not call into Godot from a driver callback, and do not panic on an I/O
    // error across this extern-system boundary. The callback retains no context.
    let _ = writeln!(std::io::stderr(), "D3D12 ERROR: [{}] {message}", id.0);
}

fn godot_command_queue(
    driver_handle: u64,
    device: &ID3D12Device,
) -> Result<ID3D12CommandQueue, String> {
    // Godot 4.6's public COMMAND_QUEUE resource is ID3D12CommandQueue*.
    // Take our own COM reference and verify it belongs to the logical device.
    // https://github.com/godotengine/godot/blob/89cea143987d564363e15d207438530651d943ac/drivers/d3d12/rendering_device_driver_d3d12.cpp#L5143-L5146
    let queue: ID3D12CommandQueue = unsafe { clone_native_interface(driver_handle) }?;
    let mut queue_device: Option<ID3D12Device> = None;
    unsafe { queue.GetDevice(&mut queue_device) }
        .map_err(|error| format!("Get Godot queue device failed: {error}"))?;
    let queue_device = queue_device.ok_or("Godot queue returned a null device")?;
    let queue_identity = queue_device
        .cast::<IUnknown>()
        .map_err(|error| format!("Query Godot queue device identity failed: {error}"))?;
    let device_identity = device
        .cast::<IUnknown>()
        .map_err(|error| format!("Query Godot logical device identity failed: {error}"))?;
    if queue_identity != device_identity {
        return Err("Godot's D3D12 queue belongs to a different logical device".into());
    }
    Ok(queue)
}

/// # Safety
/// The pointer must be a live COM interface of type T, borrowed from Godot.
unsafe fn clone_native_interface<T: Interface>(pointer: u64) -> Result<T, String> {
    let pointer = pointer as *mut c_void;
    unsafe { T::from_raw_borrowed(&pointer) }
        .cloned()
        .ok_or_else(|| "Godot returned a null native D3D12 interface".into())
}

/// Get the GPU vendor and device IDs from Godot's D3D12 device.
pub fn get_godot_gpu_device_ids() -> Option<(u32, u32)> {
    let rd = RenderingServer::singleton().get_rendering_device()?;
    let device_ptr = rd.get_driver_resource(DriverResource::LOGICAL_DEVICE, Rid::Invalid, 0);
    let device: ID3D12Device = unsafe { clone_native_interface(device_ptr) }.ok()?;
    let target_luid: LUID = unsafe { device.GetAdapterLuid() };
    let factory: IDXGIFactory = unsafe { CreateDXGIFactory() }.ok()?;
    let mut adapter_index = 0u32;
    while let Ok(adapter) = unsafe { factory.EnumAdapters(adapter_index) } {
        let adapter: IDXGIAdapter = adapter;
        adapter_index += 1;
        let Ok(desc) = (unsafe { adapter.GetDesc() }) else {
            continue;
        };
        if desc.AdapterLuid.HighPart == target_luid.HighPart
            && desc.AdapterLuid.LowPart == target_luid.LowPart
        {
            return Some((desc.VendorId, desc.DeviceId));
        }
    }
    None
}

// Native context/allocator access is serialized by the containing importer
// mutex; capture completes its GPU work before releasing that mutex.
unsafe impl Send for D3D12TextureImporter {}
unsafe impl Sync for D3D12TextureImporter {}
