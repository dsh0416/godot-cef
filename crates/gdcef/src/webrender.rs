use adblock::request::{Request as AdblockRequest, RequestError as AdblockRequestError};
use cef::{self, rc::Rc, sys::cef_cursor_type_t, *};
use cef_app::{CursorType, PhysicalSize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use wide::{i8x16, u8x16};

use crate::accelerated_osr::PlatformAcceleratedRenderHandler;
use crate::browser::{
    AudioPacket, AudioPacketQueue, AudioParamsState, AudioSampleRateState, AudioShutdownFlag,
    AudioState, ConsoleMessageEvent, DownloadRequestEvent, DownloadUpdateEvent, DragDataInfo,
    DragEvent, EventQueues, EventQueuesHandle, ImeCompositionRange, LoadingStateEvent,
    PendingPermissionAggregates, PendingPermissionDecision, PendingPermissionRequests,
    PermissionPolicyFlag, PermissionRequestEvent, PermissionRequestIdCounter,
};
use crate::utils::get_display_scale_factor;

/// Bundles all the event queues and audio state used for browser-to-Godot communication.
#[derive(Clone)]
pub(crate) struct ClientQueues {
    /// Consolidated event queues (UI-thread callbacks).
    pub event_queues: EventQueuesHandle,
    /// Audio packet queue (may be called from audio thread).
    pub audio_packet_queue: AudioPacketQueue,
    /// Audio parameters state.
    pub audio_params: AudioParamsState,
    /// Audio sample rate.
    pub audio_sample_rate: AudioSampleRateState,
    /// Audio shutdown flag.
    pub audio_shutdown_flag: AudioShutdownFlag,
    /// Whether audio capture is enabled.
    pub enable_audio_capture: bool,
    /// Default permission policy shared with the permission handler.
    pub permission_policy: PermissionPolicyFlag,
    /// Monotonic request-id counter for permission events.
    pub permission_request_counter: PermissionRequestIdCounter,
    /// Pending permission callback map keyed by request id.
    pub pending_permission_requests: PendingPermissionRequests,
    /// Aggregated permission decision state keyed by callback token.
    pub pending_permission_aggregates: PendingPermissionAggregates,
}

impl ClientQueues {
    pub fn new(
        sample_rate: f32,
        enable_audio_capture: bool,
        permission_policy: PermissionPolicyFlag,
        permission_request_counter: PermissionRequestIdCounter,
        pending_permission_requests: PendingPermissionRequests,
        pending_permission_aggregates: PendingPermissionAggregates,
    ) -> Self {
        use std::sync::atomic::AtomicBool;
        Self {
            event_queues: Arc::new(Mutex::new(EventQueues::new())),
            audio_packet_queue: Arc::new(Mutex::new(VecDeque::new())),
            audio_params: Arc::new(Mutex::new(None)),
            audio_sample_rate: Arc::new(Mutex::new(sample_rate)),
            audio_shutdown_flag: Arc::new(AtomicBool::new(false)),
            enable_audio_capture,
            permission_policy,
            permission_request_counter,
            pending_permission_requests,
            pending_permission_aggregates,
        }
    }

    /// Consumes the queues and returns an `AudioState` if audio capture is enabled,
    /// or `None` otherwise. Call this after passing a clone to the client builder.
    pub fn into_audio_state(self) -> Option<AudioState> {
        if self.enable_audio_capture {
            Some(AudioState {
                packet_queue: self.audio_packet_queue,
                sample_rate: self.audio_sample_rate,
                shutdown_flag: self.audio_shutdown_flag,
            })
        } else {
            None
        }
    }
}

/// Swizzle indices for BGRA -> RGBA conversion.
/// [B,G,R,A] at indices [0,1,2,3] -> [R,G,B,A] means pick [2,1,0,3] for each pixel.
const BGRA_TO_RGBA_INDICES: i8x16 =
    i8x16::new([2, 1, 0, 3, 6, 5, 4, 7, 10, 9, 8, 11, 14, 13, 12, 15]);

/// Converts BGRA pixel data to RGBA using SIMD operations.
/// Processes 16 bytes (4 pixels) at a time for optimal performance.
fn bgra_to_rgba(bgra: &[u8]) -> Vec<u8> {
    let mut rgba = vec![0u8; bgra.len()];

    // Process 16 bytes (4 pixels) at a time using SIMD
    let simd_chunks = bgra.len() / 16;
    for i in 0..simd_chunks {
        let offset = i * 16;
        let mut src = [0u8; 16];
        src.copy_from_slice(&bgra[offset..offset + 16]);
        let v = u8x16::new(src);
        // Swizzle BGRA -> RGBA using precomputed indices
        let shuffled = v.swizzle(BGRA_TO_RGBA_INDICES);
        let result: [i8; 16] = shuffled.into();
        let result_u8: [u8; 16] = result.map(|b| b as u8);
        rgba[offset..offset + 16].copy_from_slice(&result_u8);
    }

    // Handle remaining pixels that don't fit in a 16-byte chunk
    let remainder_start = simd_chunks * 16;
    for (src, dst) in bgra[remainder_start..]
        .chunks_exact(4)
        .zip(rgba[remainder_start..].chunks_exact_mut(4))
    {
        dst[0] = src[2]; // R
        dst[1] = src[1]; // G
        dst[2] = src[0]; // B
        dst[3] = src[3]; // A
    }

    rgba
}

/// Common helper for view_rect implementation.
fn compute_view_rect(size: &Arc<Mutex<PhysicalSize<f32>>>, rect: Option<&mut Rect>) {
    if let Some(rect) = rect
        && let Ok(size) = size.lock()
        && size.width > 0.0
        && size.height > 0.0
    {
        let scale = get_display_scale_factor();
        rect.width = (size.width / scale) as i32;
        rect.height = (size.height / scale) as i32;
    }
}

/// Common helper for screen_info implementation.
fn compute_screen_info(screen_info: Option<&mut ScreenInfo>) -> ::std::os::raw::c_int {
    if let Some(screen_info) = screen_info {
        screen_info.device_scale_factor = get_display_scale_factor();
        return true as _;
    }
    false as _
}

fn compute_screen_point(
    view_x: ::std::os::raw::c_int,
    view_y: ::std::os::raw::c_int,
    screen_x: Option<&mut ::std::os::raw::c_int>,
    screen_y: Option<&mut ::std::os::raw::c_int>,
) -> ::std::os::raw::c_int {
    if let Some(screen_x) = screen_x {
        *screen_x = view_x;
    }
    if let Some(screen_y) = screen_y {
        *screen_y = view_y;
    }
    true as _
}

fn handle_popup_show(popup_state: &Arc<Mutex<cef_app::PopupState>>, show: ::std::os::raw::c_int) {
    if let Ok(mut state) = popup_state.lock() {
        state.set_visible(show != 0);
    }
}

fn handle_popup_size(popup_state: &Arc<Mutex<cef_app::PopupState>>, rect: Option<&Rect>) {
    if let Some(rect) = rect
        && let Ok(mut state) = popup_state.lock()
    {
        state.set_rect(rect.x, rect.y, rect.width, rect.height);
    }
}

/// Helper to convert DragOperationsMask to u32 in a cross-platform way.
fn drag_ops_to_u32(ops: DragOperationsMask) -> u32 {
    #[cfg(target_os = "windows")]
    {
        ops.as_ref().0 as u32
    }
    #[cfg(not(target_os = "windows"))]
    {
        ops.as_ref().0
    }
}

/// Helper to convert MediaAccessPermissionTypes bitmask to u32 in a cross-platform way.
fn media_permission_to_u32(permission: cef::MediaAccessPermissionTypes) -> u32 {
    #[cfg(target_os = "windows")]
    {
        permission.get_raw() as u32
    }
    #[cfg(not(target_os = "windows"))]
    {
        permission.get_raw()
    }
}

/// Helper to convert PermissionRequestTypes bitmask to u32 in a cross-platform way.
fn prompt_permission_to_u32(permission: cef::PermissionRequestTypes) -> u32 {
    #[cfg(target_os = "windows")]
    {
        permission.get_raw() as u32
    }
    #[cfg(not(target_os = "windows"))]
    {
        permission.get_raw()
    }
}

fn cef_resource_type_to_adblock_request_type(resource_type: ResourceType) -> &'static str {
    match resource_type {
        ResourceType::MAIN_FRAME => "main_frame",
        ResourceType::SUB_FRAME => "sub_frame",
        ResourceType::STYLESHEET => "stylesheet",
        ResourceType::SCRIPT => "script",
        ResourceType::IMAGE => "image",
        ResourceType::FONT_RESOURCE => "font",
        ResourceType::SUB_RESOURCE => "object_subrequest",
        ResourceType::OBJECT => "object",
        ResourceType::MEDIA => "media",
        ResourceType::WORKER => "script",
        ResourceType::SHARED_WORKER => "script",
        ResourceType::PREFETCH => "other",
        ResourceType::FAVICON => "image",
        ResourceType::XHR => "xhr",
        ResourceType::PING => "ping",
        ResourceType::SERVICE_WORKER => "script",
        ResourceType::CSP_REPORT => "csp_report",
        ResourceType::PLUGIN_RESOURCE => "object",
        ResourceType::NAVIGATION_PRELOAD_MAIN_FRAME => "main_frame",
        ResourceType::NAVIGATION_PRELOAD_SUB_FRAME => "sub_frame",
        ResourceType::NUM_VALUES => "other",
        _ => "other",
    }
}

fn cef_request_to_adblock_request(
    request: &cef::Request,
) -> Result<AdblockRequest, AdblockRequestError> {
    AdblockRequest::new(
        &CefStringUtf16::from(&request.url()).to_string(),
        &CefStringUtf16::from(&request.referrer_url()).to_string(),
        cef_resource_type_to_adblock_request_type(request.resource_type()),
    )
}

/// Common helper for start_dragging implementation.
fn handle_start_dragging(
    drag_data: Option<&mut DragData>,
    allowed_ops: DragOperationsMask,
    x: ::std::os::raw::c_int,
    y: ::std::os::raw::c_int,
    event_queues: &EventQueuesHandle,
) -> ::std::os::raw::c_int {
    if let Some(drag_data) = drag_data {
        let drag_info = extract_drag_data_info(drag_data);
        if let Ok(mut queues) = event_queues.lock() {
            queues.drag_events.push_back(DragEvent::Started {
                drag_data: drag_info,
                x,
                y,
                allowed_ops: drag_ops_to_u32(allowed_ops),
            });
        }
    }
    1
}

/// Common helper for update_drag_cursor implementation.
fn handle_update_drag_cursor(operation: DragOperationsMask, event_queues: &EventQueuesHandle) {
    if let Ok(mut queues) = event_queues.lock() {
        queues.drag_events.push_back(DragEvent::UpdateCursor {
            operation: drag_ops_to_u32(operation),
        });
    }
}

wrap_render_handler! {
    pub struct SoftwareOsrHandler {
        handler: cef_app::OsrRenderHandler,
        event_queues: EventQueuesHandle,
    }

    impl RenderHandler {
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
            compute_view_rect(&self.handler.size, rect);
        }

        fn screen_info(
            &self,
            _browser: Option<&mut Browser>,
            screen_info: Option<&mut ScreenInfo>,
        ) -> ::std::os::raw::c_int {
            compute_screen_info(screen_info)
        }

        fn screen_point(
            &self,
            _browser: Option<&mut Browser>,
            view_x: ::std::os::raw::c_int,
            view_y: ::std::os::raw::c_int,
            screen_x: Option<&mut ::std::os::raw::c_int>,
            screen_y: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            compute_screen_point(view_x, view_y, screen_x, screen_y)
        }

        fn on_popup_show(
            &self,
            _browser: Option<&mut Browser>,
            show: ::std::os::raw::c_int,
        ) {
            handle_popup_show(&self.handler.popup_state, show);
        }

        fn on_popup_size(
            &self,
            _browser: Option<&mut Browser>,
            rect: Option<&Rect>,
        ) {
            handle_popup_size(&self.handler.popup_state, rect);
        }

        fn on_paint(
            &self,
            _browser: Option<&mut Browser>,
            type_: PaintElementType,
            _dirty_rects: Option<&[Rect]>,
            buffer: *const u8,
            width: ::std::os::raw::c_int,
            height: ::std::os::raw::c_int,
        ) {
            if buffer.is_null() || width <= 0 || height <= 0 {
                return;
            }

            let width = width as u32;
            let height = height as u32;
            let buffer_size = (width * height * 4) as usize;
            let bgra_data = unsafe { std::slice::from_raw_parts(buffer, buffer_size) };
            let rgba_data = bgra_to_rgba(bgra_data);

            if type_ == PaintElementType::VIEW {
                if let Ok(mut frame_buffer) = self.handler.frame_buffer.lock() {
                    frame_buffer.update(rgba_data, width, height);
                }
            } else if type_ == PaintElementType::POPUP
                && let Ok(mut popup_state) = self.handler.popup_state.lock() {
                    popup_state.update_buffer(rgba_data, width, height);
                }
        }

        fn on_ime_composition_range_changed(
            &self,
            _browser: Option<&mut Browser>,
            _selected_range: Option<&Range>,
            character_bounds: Option<&[Rect]>,
        ) {
            if let Some(bounds) = character_bounds.and_then(|b| b.last())
                && let Ok(mut queues) = self.event_queues.lock() {
                    queues.ime_composition_range = Some(ImeCompositionRange {
                        caret_x: bounds.x,
                        caret_y: bounds.y,
                        caret_height: bounds.height,
                    });
                }
        }

        fn start_dragging(
            &self,
            _browser: Option<&mut Browser>,
            drag_data: Option<&mut DragData>,
            allowed_ops: DragOperationsMask,
            x: ::std::os::raw::c_int,
            y: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            handle_start_dragging(drag_data, allowed_ops, x, y, &self.event_queues)
        }

        fn update_drag_cursor(
            &self,
            _browser: Option<&mut Browser>,
            operation: DragOperationsMask,
        ) {
            handle_update_drag_cursor(operation, &self.event_queues);
        }
    }
}

impl SoftwareOsrHandler {
    pub fn build(
        handler: cef_app::OsrRenderHandler,
        event_queues: EventQueuesHandle,
    ) -> cef::RenderHandler {
        Self::new(handler, event_queues)
    }
}

wrap_render_handler! {
    pub struct AcceleratedOsrHandler {
        handler: PlatformAcceleratedRenderHandler,
        event_queues: EventQueuesHandle,
    }

    impl RenderHandler {
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
            compute_view_rect(&self.handler.size, rect);
        }

        fn screen_info(
            &self,
            _browser: Option<&mut Browser>,
            screen_info: Option<&mut ScreenInfo>,
        ) -> ::std::os::raw::c_int {
            compute_screen_info(screen_info)
        }

        fn screen_point(
            &self,
            _browser: Option<&mut Browser>,
            view_x: ::std::os::raw::c_int,
            view_y: ::std::os::raw::c_int,
            screen_x: Option<&mut ::std::os::raw::c_int>,
            screen_y: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            compute_screen_point(view_x, view_y, screen_x, screen_y)
        }

        fn on_popup_show(
            &self,
            _browser: Option<&mut Browser>,
            show: ::std::os::raw::c_int,
        ) {
            handle_popup_show(&self.handler.popup_state, show);
        }

        fn on_popup_size(
            &self,
            _browser: Option<&mut Browser>,
            rect: Option<&Rect>,
        ) {
            handle_popup_size(&self.handler.popup_state, rect);
        }

        fn on_accelerated_paint(
            &self,
            _browser: Option<&mut Browser>,
            type_: PaintElementType,
            _dirty_rects: Option<&[Rect]>,
            info: Option<&AcceleratedPaintInfo>,
        ) {
            self.handler.on_accelerated_paint(type_, info);
        }

        fn on_paint(
            &self,
            _browser: Option<&mut Browser>,
            type_: PaintElementType,
            _dirty_rects: Option<&[Rect]>,
            buffer: *const u8,
            width: ::std::os::raw::c_int,
            height: ::std::os::raw::c_int,
        ) {
            if type_ == PaintElementType::POPUP
                && !buffer.is_null()
                && width > 0
                && height > 0
            {
                let width = width as u32;
                let height = height as u32;
                let buffer_size = (width * height * 4) as usize;
                let bgra_data = unsafe { std::slice::from_raw_parts(buffer, buffer_size) };
                let rgba_data = bgra_to_rgba(bgra_data);

                if let Ok(mut popup_state) = self.handler.popup_state.lock() {
                    popup_state.update_buffer(rgba_data, width, height);
                }
            }
        }

        fn on_ime_composition_range_changed(
            &self,
            _browser: Option<&mut Browser>,
            _selected_range: Option<&Range>,
            character_bounds: Option<&[Rect]>,
        ) {
            if let Some(bounds) = character_bounds.and_then(|b| b.last())
                && let Ok(mut queues) = self.event_queues.lock() {
                    queues.ime_composition_range = Some(ImeCompositionRange {
                        caret_x: bounds.x,
                        caret_y: bounds.y,
                        caret_height: bounds.height,
                    });
                }
        }

        fn start_dragging(
            &self,
            _browser: Option<&mut Browser>,
            drag_data: Option<&mut DragData>,
            allowed_ops: DragOperationsMask,
            x: ::std::os::raw::c_int,
            y: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            handle_start_dragging(drag_data, allowed_ops, x, y, &self.event_queues)
        }

        fn update_drag_cursor(
            &self,
            _browser: Option<&mut Browser>,
            operation: DragOperationsMask,
        ) {
            handle_update_drag_cursor(operation, &self.event_queues);
        }
    }
}

impl AcceleratedOsrHandler {
    pub fn build(
        handler: PlatformAcceleratedRenderHandler,
        event_queues: EventQueuesHandle,
    ) -> cef::RenderHandler {
        Self::new(handler, event_queues)
    }
}

fn cef_cursor_to_cursor_type(cef_type: cef::sys::cef_cursor_type_t) -> CursorType {
    match cef_type {
        cef_cursor_type_t::CT_POINTER => CursorType::Arrow,
        cef_cursor_type_t::CT_IBEAM => CursorType::IBeam,
        cef_cursor_type_t::CT_HAND => CursorType::Hand,
        cef_cursor_type_t::CT_CROSS => CursorType::Cross,
        cef_cursor_type_t::CT_WAIT => CursorType::Wait,
        cef_cursor_type_t::CT_HELP => CursorType::Help,
        cef_cursor_type_t::CT_MOVE => CursorType::Move,
        cef_cursor_type_t::CT_NORTHRESIZE
        | cef_cursor_type_t::CT_SOUTHRESIZE
        | cef_cursor_type_t::CT_NORTHSOUTHRESIZE => CursorType::ResizeNS,
        cef_cursor_type_t::CT_EASTRESIZE
        | cef_cursor_type_t::CT_WESTRESIZE
        | cef_cursor_type_t::CT_EASTWESTRESIZE => CursorType::ResizeEW,
        cef_cursor_type_t::CT_NORTHEASTRESIZE
        | cef_cursor_type_t::CT_SOUTHWESTRESIZE
        | cef_cursor_type_t::CT_NORTHEASTSOUTHWESTRESIZE => CursorType::ResizeNESW,
        cef_cursor_type_t::CT_NORTHWESTRESIZE
        | cef_cursor_type_t::CT_SOUTHEASTRESIZE
        | cef_cursor_type_t::CT_NORTHWESTSOUTHEASTRESIZE => CursorType::ResizeNWSE,
        cef_cursor_type_t::CT_NOTALLOWED => CursorType::NotAllowed,
        cef_cursor_type_t::CT_PROGRESS => CursorType::Progress,
        _ => CursorType::Arrow,
    }
}

macro_rules! handle_cursor_change {
    ($self:expr, $type_:expr) => {{
        let cursor = cef_cursor_to_cursor_type($type_.into());
        if let Ok(mut ct) = $self.cursor_type.lock() {
            *ct = cursor;
        }
        false as i32
    }};
}

fn extract_drag_data_info(drag_data: &impl ImplDragData) -> DragDataInfo {
    let is_link = drag_data.is_link() != 0;
    let is_file = drag_data.is_file() != 0;
    let is_fragment = drag_data.is_fragment() != 0;

    let link_url = if is_link {
        let s = drag_data.link_url();
        CefStringUtf16::from(&s).to_string()
    } else {
        String::new()
    };

    let link_title = if is_link {
        let s = drag_data.link_title();
        CefStringUtf16::from(&s).to_string()
    } else {
        String::new()
    };

    let fragment_text = if is_fragment {
        let s = drag_data.fragment_text();
        CefStringUtf16::from(&s).to_string()
    } else {
        String::new()
    };

    let fragment_html = if is_fragment {
        let s = drag_data.fragment_html();
        CefStringUtf16::from(&s).to_string()
    } else {
        String::new()
    };

    let file_names = if is_file {
        let name = drag_data.file_name();
        let name_str = CefStringUtf16::from(&name).to_string();
        if name_str.is_empty() {
            Vec::new()
        } else {
            vec![name_str]
        }
    } else {
        Vec::new()
    };

    DragDataInfo {
        is_link,
        is_file,
        is_fragment,
        link_url,
        link_title,
        fragment_text,
        fragment_html,
        file_names,
    }
}

wrap_drag_handler! {
    pub(crate) struct DragHandlerImpl {
        event_queues: EventQueuesHandle,
    }

    impl DragHandler {
        fn on_drag_enter(
            &self,
            _browser: Option<&mut Browser>,
            drag_data: Option<&mut DragData>,
            mask: DragOperationsMask,
        ) -> ::std::os::raw::c_int {
            if let Some(drag_data) = drag_data {
                let drag_info = extract_drag_data_info(drag_data);
                if let Ok(mut queues) = self.event_queues.lock() {
                    #[cfg(target_os = "windows")]
                    let mask: u32 = mask.as_ref().0 as u32;
                    #[cfg(not(target_os = "windows"))]
                    let mask: u32 = mask.as_ref().0;

                    queues.drag_events.push_back(DragEvent::Entered {
                        drag_data: drag_info,
                        mask,
                    });
                }
            }
            0
        }
    }
}

impl DragHandlerImpl {
    pub fn build(event_queues: EventQueuesHandle) -> cef::DragHandler {
        Self::new(event_queues)
    }
}

wrap_display_handler! {
    pub(crate) struct DisplayHandlerImpl {
        cursor_type: Arc<Mutex<CursorType>>,
        event_queues: EventQueuesHandle,
    }

    impl DisplayHandler {
        #[cfg(target_os = "windows")]
        fn on_cursor_change(
            &self,
            _browser: Option<&mut Browser>,
            _cursor: *mut cef::sys::HICON__,
            type_: cef::CursorType,
            _custom_cursor_info: Option<&CursorInfo>,
        ) -> i32 {
            handle_cursor_change!(self, type_)
        }

        #[cfg(target_os = "macos")]
        fn on_cursor_change(
            &self,
            _browser: Option<&mut Browser>,
            _cursor: *mut u8,
            type_: cef::CursorType,
            _custom_cursor_info: Option<&CursorInfo>,
        ) -> i32 {
            handle_cursor_change!(self, type_)
        }

        #[cfg(target_os = "linux")]
        fn on_cursor_change(
            &self,
            _browser: Option<&mut Browser>,
            _cursor: u64,
            type_: cef::CursorType,
            _custom_cursor_info: Option<&CursorInfo>,
        ) -> i32 {
            handle_cursor_change!(self, type_)
        }

        fn on_address_change(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            url: Option<&CefString>,
        ) {
            if let Some(url) = url {
                let url_str = url.to_string();
                if let Ok(mut queues) = self.event_queues.lock() {
                    queues.url_changes.push_back(url_str);
                }
            }
        }

        fn on_title_change(
            &self,
            _browser: Option<&mut Browser>,
            title: Option<&CefString>,
        ) {
            if let Some(title) = title {
                let title_str = title.to_string();
                if let Ok(mut queues) = self.event_queues.lock() {
                    queues.title_changes.push_back(title_str);
                }
            }
        }

        fn on_console_message(
            &self,
            _browser: Option<&mut Browser>,
            level: cef::LogSeverity,
            message: Option<&CefString>,
            source: Option<&CefString>,
            line: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            let message_str = message.map(|m| m.to_string()).unwrap_or_default();
            let source_str = source.map(|s| s.to_string()).unwrap_or_default();
            #[cfg(target_os = "windows")]
            let level: u32 = level.get_raw() as u32;
            #[cfg(not(target_os = "windows"))]
            let level: u32 = level.get_raw();

            if let Ok(mut queues) = self.event_queues.lock() {
                queues.console_messages.push_back(ConsoleMessageEvent {
                    level,
                    message: message_str,
                    source: source_str,
                    line,
                });
            }

            // Return false to allow default console output
            false as _
        }
    }
}

impl DisplayHandlerImpl {
    pub fn build(
        cursor_type: Arc<Mutex<CursorType>>,
        event_queues: EventQueuesHandle,
    ) -> cef::DisplayHandler {
        Self::new(cursor_type, event_queues)
    }
}

wrap_context_menu_handler! {
    pub(crate) struct ContextMenuHandlerImpl {}

    impl ContextMenuHandler {
        fn on_before_context_menu(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _params: Option<&mut ContextMenuParams>,
            model: Option<&mut MenuModel>,
        ) {
            if let Some(model) = model {
                model.clear();
            }
        }
    }
}

impl ContextMenuHandlerImpl {
    pub fn build() -> cef::ContextMenuHandler {
        Self::new()
    }
}

wrap_life_span_handler! {
    pub(crate) struct LifeSpanHandlerImpl {
        event_queues: EventQueuesHandle,
        popup_policy: crate::browser::PopupPolicyFlag,
    }

    impl LifeSpanHandler {
        fn on_before_popup(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _popup_id: ::std::os::raw::c_int,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            target_disposition: WindowOpenDisposition,
            user_gesture: ::std::os::raw::c_int,
            _popup_features: Option<&PopupFeatures>,
            _window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            use crate::browser::{popup_policy, PopupRequestEvent};
            use std::sync::atomic::Ordering;

            let policy = self.popup_policy.load(Ordering::Relaxed);
            let url = target_url
                .map(|u| u.to_string())
                .unwrap_or_default();

            match policy {
                popup_policy::REDIRECT => {
                    // Navigate the current browser to the popup URL
                    if let Some(browser) = browser
                        && let Some(frame) = browser.main_frame()
                        && !url.is_empty()
                    {
                        let url_cef = CefStringUtf16::from(url.as_str());
                        frame.load_url(Some(&url_cef));
                    }
                    // Return true to cancel the popup (we handled it via navigation)
                    true as _
                }
                popup_policy::SIGNAL_ONLY => {
                    // Queue the event for GDScript to handle
                    if let Ok(mut queues) = self.event_queues.lock() {
                        queues.popup_requests.push_back(PopupRequestEvent {
                            target_url: url,
                            disposition: target_disposition,
                            user_gesture: user_gesture != 0,
                        });
                    }
                    // Return true to cancel the popup (GDScript decides what to do)
                    true as _
                }
                _ => {
                    // BLOCK (default): suppress silently
                    true as _
                }
            }
        }
    }
}

impl LifeSpanHandlerImpl {
    pub fn build(
        event_queues: EventQueuesHandle,
        popup_policy: crate::browser::PopupPolicyFlag,
    ) -> cef::LifeSpanHandler {
        Self::new(event_queues, popup_policy)
    }
}

wrap_load_handler! {
    pub(crate) struct LoadHandlerImpl {
        event_queues: EventQueuesHandle,
    }

    impl LoadHandler {
        fn on_load_start(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            _transition_type: TransitionType,
        ) {
            if let Some(frame) = frame
                && frame.is_main() != 0
            {
                let url = CefStringUtf16::from(&frame.url()).to_string();
                if let Ok(mut queues) = self.event_queues.lock() {
                    queues.loading_states.push_back(LoadingStateEvent::Started { url });
                }
            }
        }

        fn on_load_end(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            http_status_code: ::std::os::raw::c_int,
        ) {
            if let Some(frame) = frame
                && frame.is_main() != 0
            {
                let url = CefStringUtf16::from(&frame.url()).to_string();
                if let Ok(mut queues) = self.event_queues.lock() {
                    queues.loading_states.push_back(LoadingStateEvent::Finished {
                        url,
                        http_status_code,
                    });
                }
            }
        }

        fn on_load_error(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            error_code: Errorcode,
            error_string: Option<&CefString>,
            failed_url: Option<&CefString>,
        ) {
            if let Some(frame) = frame
                && frame.is_main() != 0
            {
                let url = failed_url
                    .map(|u| u.to_string())
                    .unwrap_or_default();
                let error_text = error_string
                    .map(|e| e.to_string())
                    .unwrap_or_default();
                // Use the get_raw() method to safely convert Errorcode to i32
                let error_code_i32: i32 = error_code.get_raw();
                if let Ok(mut queues) = self.event_queues.lock() {
                    queues.loading_states.push_back(LoadingStateEvent::Error {
                        url,
                        error_code: error_code_i32,
                        error_text,
                    });
                }
            }
        }
    }
}

impl LoadHandlerImpl {
    pub fn build(event_queues: EventQueuesHandle) -> cef::LoadHandler {
        Self::new(event_queues)
    }
}

wrap_audio_handler! {
    pub(crate) struct AudioHandlerImpl {
        audio_params: AudioParamsState,
        audio_packet_queue: AudioPacketQueue,
        audio_sample_rate: AudioSampleRateState,
        audio_shutdown_flag: AudioShutdownFlag,
    }

    impl AudioHandler {
        fn audio_parameters(
            &self,
            _browser: Option<&mut Browser>,
            params: Option<&mut cef::AudioParameters>,
        ) -> ::std::os::raw::c_int {
            if let Some(params) = params {
                let sample_rate = self.audio_sample_rate
                    .lock()
                    .map(|sr| *sr)
                    .unwrap_or(48000.0);

                params.channel_layout = ChannelLayout::LAYOUT_STEREO;
                params.sample_rate = sample_rate as i32;
                params.frames_per_buffer = 256;
            }
            true as _
        }

        fn on_audio_stream_started(
            &self,
            _browser: Option<&mut Browser>,
            params: Option<&cef::AudioParameters>,
            channels: ::std::os::raw::c_int,
        ) {
            if let Some(params) = params
                && let Ok(mut audio_params) = self.audio_params.lock()
            {
                *audio_params = Some(crate::browser::AudioParameters {
                    channels,
                    sample_rate: params.sample_rate,
                    frames_per_buffer: params.frames_per_buffer,
                });
            }
        }

        fn on_audio_stream_packet(
            &self,
            _browser: Option<&mut Browser>,
            data: *mut *const f32,
            frames: ::std::os::raw::c_int,
            pts: i64,
        ) {
            if data.is_null() || frames <= 0 {
                return;
            }

            let channels = self.audio_params
                .lock()
                .ok()
                .and_then(|p| p.as_ref().map(|a| a.channels))
                .unwrap_or(2);

            if channels != 2 {
                godot::global::godot_error!(
                    "[CefAudioHandler] Expected 2 audio channels (stereo), but got {}. Dropping audio packet.",
                    channels
                );
                return;
            }
            let mut interleaved = Vec::with_capacity((frames * channels) as usize);

            unsafe {
                for frame_idx in 0..frames as isize {
                    for ch in 0..channels as isize {
                        let channel_ptr = *data.offset(ch);
                        if !channel_ptr.is_null() {
                            interleaved.push(*channel_ptr.offset(frame_idx));
                        } else {
                            interleaved.push(0.0);
                        }
                    }
                }
            }

            if let Ok(mut queue) = self.audio_packet_queue.lock() {
                const MAX_QUEUE_SIZE: usize = 100;
                while queue.len() >= MAX_QUEUE_SIZE {
                    queue.pop_front();
                }
                queue.push_back(AudioPacket {
                    data: interleaved,
                    frames,
                    pts,
                });
            }
        }

        fn on_audio_stream_stopped(&self, _browser: Option<&mut Browser>) {
            if let Ok(mut queue) = self.audio_packet_queue.lock() {
                queue.clear();
            }
            if let Ok(mut params) = self.audio_params.lock() {
                *params = None;
            }
        }

        fn on_audio_stream_error(
            &self,
            _browser: Option<&mut Browser>,
            message: Option<&CefString>,
        ) {
            use std::sync::atomic::Ordering;
            if self.audio_shutdown_flag.load(Ordering::Relaxed) {
                return;
            }
            if let Some(msg) = message {
                let msg_str = msg.to_string();
                godot::global::godot_error!("[CefAudioHandler] Audio stream error: {}", msg_str);
            }
        }
    }
}

impl AudioHandlerImpl {
    pub fn build(
        audio_params: AudioParamsState,
        audio_packet_queue: AudioPacketQueue,
        audio_sample_rate: AudioSampleRateState,
        audio_shutdown_flag: AudioShutdownFlag,
    ) -> cef::AudioHandler {
        Self::new(
            audio_params,
            audio_packet_queue,
            audio_sample_rate,
            audio_shutdown_flag,
        )
    }
}

wrap_download_handler! {
    pub(crate) struct DownloadHandlerImpl {
        event_queues: EventQueuesHandle,
    }

    impl DownloadHandler {
        fn can_download(
            &self,
            _browser: Option<&mut Browser>,
            _url: Option<&CefString>,
            _request_method: Option<&CefString>,
        ) -> ::std::os::raw::c_int {
            true as _
        }

        fn on_before_download(
            &self,
            _browser: Option<&mut Browser>,
            download_item: Option<&mut cef::DownloadItem>,
            suggested_name: Option<&CefString>,
            callback: Option<&mut cef::BeforeDownloadCallback>,
        ) -> ::std::os::raw::c_int {
            if let Some(item) = download_item {
                let url = CefStringUtf16::from(&item.url()).to_string();
                let original_url = CefStringUtf16::from(&item.original_url()).to_string();
                let suggested_file_name = suggested_name
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                let mime_type = CefStringUtf16::from(&item.mime_type()).to_string();
                let total_bytes = item.total_bytes();
                let id = item.id();

                if let Ok(mut queues) = self.event_queues.lock() {
                    queues.download_requests.push_back(DownloadRequestEvent {
                        id,
                        url,
                        original_url,
                        suggested_file_name,
                        mime_type,
                        total_bytes,
                    });
                }

                if let Some(callback) = callback {
                    let empty_path: cef::CefStringUtf16 = "".into();
                    callback.cont(Some(&empty_path), 0);
                }
            }
            false as _
        }

        fn on_download_updated(
            &self,
            _browser: Option<&mut Browser>,
            download_item: Option<&mut cef::DownloadItem>,
            _callback: Option<&mut cef::DownloadItemCallback>,
        ) {
            if let Some(item) = download_item {
                let id = item.id();
                let url = CefStringUtf16::from(&item.url()).to_string();
                let full_path = CefStringUtf16::from(&item.full_path()).to_string();
                let received_bytes = item.received_bytes();
                let total_bytes = item.total_bytes();
                let current_speed = item.current_speed();
                let percent_complete = item.percent_complete();
                let is_in_progress = item.is_in_progress() != 0;
                let is_complete = item.is_complete() != 0;
                let is_canceled = item.is_canceled() != 0;

                if let Ok(mut queues) = self.event_queues.lock() {
                    queues.download_updates.push_back(DownloadUpdateEvent {
                        id,
                        url,
                        full_path,
                        received_bytes,
                        total_bytes,
                        current_speed,
                        percent_complete,
                        is_in_progress,
                        is_complete,
                        is_canceled,
                    });
                }
            }
        }
    }
}

impl DownloadHandlerImpl {
    pub fn build(event_queues: EventQueuesHandle) -> cef::DownloadHandler {
        Self::new(event_queues)
    }
}

wrap_request_handler! {
    pub(crate) struct RequestHandlerImpl {
        event_queues: EventQueuesHandle,
    }

    impl RequestHandler {
        fn on_render_process_terminated(
            &self,
            _browser: Option<&mut Browser>,
            status: cef::TerminationStatus,
            _error_code: i32,
            _error_string: Option<&cef::CefStringUtf16>,
        ) {
            let reason = match status {
                cef::TerminationStatus::ABNORMAL_TERMINATION => "Abnormal Termination",
                cef::TerminationStatus::PROCESS_WAS_KILLED => "Process Was Killed",
                cef::TerminationStatus::PROCESS_CRASHED => "Process Crashed",
                cef::TerminationStatus::PROCESS_OOM => "Process OOM",
                _ => "Unknown",
            };

            if let Ok(mut queues) = self.event_queues.lock() {
                queues.render_process_terminated.push_back((reason.to_string(), status));
            }
        }
    }
}

impl RequestHandlerImpl {
    pub fn build(event_queues: EventQueuesHandle) -> cef::RequestHandler {
        Self::new(event_queues)
    }
}

fn push_permission_request(
    event_queues: &EventQueuesHandle,
    pending_permission_requests: &PendingPermissionRequests,
    permission_request_counter: &PermissionRequestIdCounter,
    pending_decision: PendingPermissionDecision,
    permission_type: String,
    url: String,
) {
    use std::sync::atomic::Ordering;

    let request_id = permission_request_counter.fetch_add(1, Ordering::Relaxed) + 1;

    if let Ok(mut pending) = pending_permission_requests.lock() {
        pending.insert(request_id, pending_decision);
    } else {
        return;
    }

    if let Ok(mut queues) = event_queues.lock() {
        queues
            .permission_requests
            .push_back(PermissionRequestEvent {
                permission_type,
                url,
                request_id,
            });
    } else if let Ok(mut pending) = pending_permission_requests.lock() {
        pending.remove(&request_id);
    }
}

fn map_media_permission_types(requested_permissions: u32) -> Vec<(u32, &'static str)> {
    let mappings = [
        (
            media_permission_to_u32(cef::MediaAccessPermissionTypes::DEVICE_AUDIO_CAPTURE),
            "microphone",
        ),
        (
            media_permission_to_u32(cef::MediaAccessPermissionTypes::DEVICE_VIDEO_CAPTURE),
            "camera",
        ),
        (
            media_permission_to_u32(cef::MediaAccessPermissionTypes::DESKTOP_AUDIO_CAPTURE),
            "desktop_audio_capture",
        ),
        (
            media_permission_to_u32(cef::MediaAccessPermissionTypes::DESKTOP_VIDEO_CAPTURE),
            "desktop_video_capture",
        ),
    ];

    let mut out = Vec::new();
    let mut known_mask = 0u32;
    for (bit, label) in mappings {
        known_mask |= bit;
        if requested_permissions & bit != 0 {
            out.push((bit, label));
        }
    }

    let unknown = requested_permissions & !known_mask;
    if unknown != 0 {
        out.push((unknown, "unknown_media_permission"));
    }

    if out.is_empty() {
        out.push((requested_permissions, "unknown_media_permission"));
    }

    out
}

fn map_prompt_permission_types(requested_permissions: u32) -> Vec<&'static str> {
    let mappings = [
        (
            prompt_permission_to_u32(cef::PermissionRequestTypes::CAMERA_STREAM),
            "camera",
        ),
        (
            prompt_permission_to_u32(cef::PermissionRequestTypes::MIC_STREAM),
            "microphone",
        ),
        (
            prompt_permission_to_u32(cef::PermissionRequestTypes::GEOLOCATION),
            "geolocation",
        ),
        (
            prompt_permission_to_u32(cef::PermissionRequestTypes::CLIPBOARD),
            "clipboard",
        ),
        (
            prompt_permission_to_u32(cef::PermissionRequestTypes::NOTIFICATIONS),
            "notifications",
        ),
        (
            prompt_permission_to_u32(cef::PermissionRequestTypes::MIDI_SYSEX),
            "midi_sysex",
        ),
        (
            prompt_permission_to_u32(cef::PermissionRequestTypes::POINTER_LOCK),
            "pointer_lock",
        ),
        (
            prompt_permission_to_u32(cef::PermissionRequestTypes::KEYBOARD_LOCK),
            "keyboard_lock",
        ),
    ];

    let mut out = Vec::new();
    let mut known_mask = 0u32;
    for (bit, label) in mappings {
        known_mask |= bit;
        if requested_permissions & bit != 0 {
            out.push(label);
        }
    }

    if requested_permissions & !known_mask != 0 {
        out.push("unknown_permission");
    }

    if out.is_empty() {
        out.push("unknown_permission");
    }

    out
}

#[cfg(test)]
mod permission_mapping_tests {
    use super::*;

    #[test]
    fn prompt_permissions_empty_defaults_to_unknown() {
        let res = map_prompt_permission_types(0);
        assert_eq!(res, vec!["unknown_permission"]);
    }

    #[test]
    fn prompt_permissions_unknown_bit_includes_unknown() {
        // Use a high bit that is very unlikely to collide with known permission bits.
        let res = map_prompt_permission_types(1u32 << 31);
        assert!(res.contains(&"unknown_permission"));
    }

    #[test]
    fn media_permissions_empty_defaults_to_unknown() {
        let res = map_media_permission_types(0);
        assert_eq!(res, vec![(0, "unknown_media_permission")]);
    }

    #[test]
    fn media_permissions_unknown_bit_includes_unknown() {
        // Use a high bit that is very unlikely to collide with known permission bits.
        let res = map_media_permission_types(1u32 << 31);
        assert!(
            res.iter()
                .any(|(_, label)| *label == "unknown_media_permission")
        );
    }
}
wrap_permission_handler! {
    pub(crate) struct PermissionHandlerImpl {
        event_queues: EventQueuesHandle,
        pending_permission_requests: PendingPermissionRequests,
        pending_permission_aggregates: PendingPermissionAggregates,
        permission_request_counter: PermissionRequestIdCounter,
        permission_policy: PermissionPolicyFlag,
    }

    impl PermissionHandler {
        fn on_request_media_access_permission(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            requesting_origin: Option<&CefString>,
            requested_permissions: u32,
            callback: Option<&mut MediaAccessCallback>,
        ) -> ::std::os::raw::c_int {
            use crate::browser::permission_policy;
            use std::sync::atomic::Ordering;

            let Some(callback) = callback else {
                return false as _;
            };
            let callback = callback.clone();
            let policy = self.permission_policy.load(Ordering::Relaxed);

            if policy == permission_policy::ALLOW_ALL {
                callback.cont(requested_permissions);
                return true as _;
            }

            if policy == permission_policy::DENY_ALL {
                callback.cont(media_permission_to_u32(cef::MediaAccessPermissionTypes::NONE));
                return true as _;
            }

            let url = requesting_origin
                .map(|origin| origin.to_string())
                .unwrap_or_default();
            let callback_token = callback.get_raw() as usize;

            for (permission_bit, permission_type) in map_media_permission_types(requested_permissions) {
                push_permission_request(
                    &self.event_queues,
                    &self.pending_permission_requests,
                    &self.permission_request_counter,
                    PendingPermissionDecision::Media {
                        callback: callback.clone(),
                        permission_bit,
                        callback_token,
                    },
                    permission_type.to_string(),
                    url.clone(),
                );
            }

            true as _
        }

        fn on_show_permission_prompt(
            &self,
            _browser: Option<&mut Browser>,
            prompt_id: u64,
            requesting_origin: Option<&CefString>,
            requested_permissions: u32,
            callback: Option<&mut PermissionPromptCallback>,
        ) -> ::std::os::raw::c_int {
            use crate::browser::permission_policy;
            use std::sync::atomic::Ordering;

            let Some(callback) = callback else {
                return false as _;
            };
            let callback = callback.clone();
            let policy = self.permission_policy.load(Ordering::Relaxed);

            if policy == permission_policy::ALLOW_ALL {
                callback.cont(cef::PermissionRequestResult::ACCEPT);
                return true as _;
            }

            if policy == permission_policy::DENY_ALL {
                callback.cont(cef::PermissionRequestResult::DENY);
                return true as _;
            }

            let url = requesting_origin
                .map(|origin| origin.to_string())
                .unwrap_or_default();
            let callback_token = callback.get_raw() as usize;
            for permission_type in map_prompt_permission_types(requested_permissions) {
                push_permission_request(
                    &self.event_queues,
                    &self.pending_permission_requests,
                    &self.permission_request_counter,
                    PendingPermissionDecision::Prompt {
                        callback: callback.clone(),
                        prompt_id,
                        callback_token,
                    },
                    permission_type.to_string(),
                    url.clone(),
                );
            }

            true as _
        }

        fn on_dismiss_permission_prompt(
            &self,
            _browser: Option<&mut Browser>,
            prompt_id: u64,
            _result: PermissionRequestResult,
        ) {
            if let Ok(mut pending) = self.pending_permission_requests.lock() {
                let mut callback_tokens = Vec::new();
                pending.retain(
                    |_, entry| match entry {
                        PendingPermissionDecision::Prompt {
                            prompt_id: id,
                            callback_token,
                            ..
                        } if *id == prompt_id => {
                            callback_tokens.push(*callback_token);
                            false
                        }
                        _ => true,
                    },
                );

                if !callback_tokens.is_empty()
                    && let Ok(mut aggregates) = self.pending_permission_aggregates.lock()
                {
                    for token in callback_tokens {
                        aggregates.remove(&token);
                    }
                }
            }
        }
    }
}

impl PermissionHandlerImpl {
    pub fn build(
        event_queues: EventQueuesHandle,
        pending_permission_requests: PendingPermissionRequests,
        pending_permission_aggregates: PendingPermissionAggregates,
        permission_request_counter: PermissionRequestIdCounter,
        permission_policy: PermissionPolicyFlag,
    ) -> cef::PermissionHandler {
        Self::new(
            event_queues,
            pending_permission_requests,
            pending_permission_aggregates,
            permission_request_counter,
            permission_policy,
        )
    }
}

fn on_process_message_received(message: Option<&mut ProcessMessage>, ipc: &ClientIpcQueues) -> i32 {
    let Some(message) = message else { return 0 };
    let route = CefStringUtf16::from(&message.name()).to_string();

    match route.as_str() {
        "ipcRendererToGodot" => {
            if let Some(args) = message.argument_list() {
                let arg = args.string(0);
                let msg_str = CefStringUtf16::from(&arg).to_string();
                if let Ok(mut queues) = ipc.event_queues.lock() {
                    queues.messages.push_back(msg_str);
                }
            }
        }
        "ipcBinaryRendererToGodot" => {
            if let Some(args) = message.argument_list()
                && let Some(binary_value) = args.binary(0)
            {
                let size = binary_value.size();
                if size > 0 {
                    let mut buffer = vec![0u8; size];
                    let copied = binary_value.data(Some(&mut buffer), 0);
                    if copied > 0 {
                        buffer.truncate(copied);
                        if let Ok(mut queues) = ipc.event_queues.lock() {
                            queues.binary_messages.push_back(buffer);
                        }
                    }
                }
            }
        }
        "triggerIme" => {
            if let Some(args) = message.argument_list() {
                let arg = args.bool(0);
                let enabled = arg != 0;
                if let Ok(mut queues) = ipc.event_queues.lock() {
                    queues.ime_enables.push_back(enabled);
                }
            }
        }
        "imeCaretPosition" => {
            if let Some(args) = message.argument_list() {
                let x = args.int(0);
                let y = args.int(1);
                let height = args.int(2);
                if let Ok(mut queues) = ipc.event_queues.lock() {
                    queues.ime_composition_range = Some(ImeCompositionRange {
                        caret_x: x,
                        caret_y: y,
                        caret_height: height,
                    });
                }
            }
        }
        _ => {}
    }

    0
}

#[derive(Clone)]
pub(crate) struct ClientHandlers {
    pub render_handler: cef::RenderHandler,
    pub display_handler: cef::DisplayHandler,
    pub context_menu_handler: cef::ContextMenuHandler,
    pub life_span_handler: cef::LifeSpanHandler,
    pub load_handler: cef::LoadHandler,
    pub drag_handler: cef::DragHandler,
    pub audio_handler: Option<cef::AudioHandler>,
    pub download_handler: cef::DownloadHandler,
    pub request_handler: cef::RequestHandler,
    pub permission_handler: cef::PermissionHandler,
}

#[derive(Clone)]
pub(crate) struct ClientIpcQueues {
    pub event_queues: EventQueuesHandle,
}

fn build_ipc_queues(queues: &ClientQueues) -> ClientIpcQueues {
    ClientIpcQueues {
        event_queues: queues.event_queues.clone(),
    }
}

wrap_client! {
    pub(crate) struct CefClientImpl {
        handlers: ClientHandlers,
        ipc: ClientIpcQueues,
    }

    impl Client {
        fn render_handler(&self) -> Option<cef::RenderHandler> {
            Some(self.handlers.render_handler.clone())
        }

        fn display_handler(&self) -> Option<cef::DisplayHandler> {
            Some(self.handlers.display_handler.clone())
        }

        fn context_menu_handler(&self) -> Option<cef::ContextMenuHandler> {
            Some(self.handlers.context_menu_handler.clone())
        }

        fn life_span_handler(&self) -> Option<cef::LifeSpanHandler> {
            Some(self.handlers.life_span_handler.clone())
        }

        fn load_handler(&self) -> Option<cef::LoadHandler> {
            Some(self.handlers.load_handler.clone())
        }

        fn drag_handler(&self) -> Option<cef::DragHandler> {
            Some(self.handlers.drag_handler.clone())
        }

        fn audio_handler(&self) -> Option<cef::AudioHandler> {
            self.handlers.audio_handler.clone()
        }

        fn download_handler(&self) -> Option<cef::DownloadHandler> {
            Some(self.handlers.download_handler.clone())
        }

        fn request_handler(&self) -> Option<cef::RequestHandler> {
            Some(self.handlers.request_handler.clone())
        }

        fn permission_handler(&self) -> Option<cef::PermissionHandler> {
            Some(self.handlers.permission_handler.clone())
        }

        fn on_process_message_received(
            &self,
            _browser: Option<&mut cef::Browser>,
            _frame: Option<&mut cef::Frame>,
            _source_process: ProcessId,
            message: Option<&mut ProcessMessage>,
        ) -> i32 {
            on_process_message_received(message, &self.ipc)
        }
    }
}

fn build_client_handlers(
    render_handler: cef::RenderHandler,
    cursor_type: Arc<Mutex<CursorType>>,
    queues: &ClientQueues,
    popup_policy: crate::browser::PopupPolicyFlag,
) -> ClientHandlers {
    let audio_handler = if queues.enable_audio_capture {
        Some(AudioHandlerImpl::build(
            queues.audio_params.clone(),
            queues.audio_packet_queue.clone(),
            queues.audio_sample_rate.clone(),
            queues.audio_shutdown_flag.clone(),
        ))
    } else {
        None
    };

    ClientHandlers {
        render_handler,
        display_handler: DisplayHandlerImpl::build(cursor_type, queues.event_queues.clone()),
        context_menu_handler: ContextMenuHandlerImpl::build(),
        life_span_handler: LifeSpanHandlerImpl::build(queues.event_queues.clone(), popup_policy),
        load_handler: LoadHandlerImpl::build(queues.event_queues.clone()),
        drag_handler: DragHandlerImpl::build(queues.event_queues.clone()),
        audio_handler,
        download_handler: DownloadHandlerImpl::build(queues.event_queues.clone()),
        request_handler: RequestHandlerImpl::build(queues.event_queues.clone()),
        permission_handler: PermissionHandlerImpl::build(
            queues.event_queues.clone(),
            queues.pending_permission_requests.clone(),
            queues.pending_permission_aggregates.clone(),
            queues.permission_request_counter.clone(),
            queues.permission_policy.clone(),
        ),
    }
}

impl CefClientImpl {
    /// Builds a CEF client from a pre-built render handler and shared queues.
    ///
    /// Both software and accelerated rendering paths use this single entry point;
    /// only the `render_handler` differs.
    pub(crate) fn build(
        render_handler: cef::RenderHandler,
        cursor_type: Arc<Mutex<CursorType>>,
        queues: ClientQueues,
        popup_policy: crate::browser::PopupPolicyFlag,
    ) -> cef::Client {
        let ipc = build_ipc_queues(&queues);
        let handlers = build_client_handlers(render_handler, cursor_type, &queues, popup_policy);
        Self::new(handlers, ipc)
    }
}

type AdblockEngineHandle = std::rc::Rc<adblock::Engine>;

#[derive(Clone)]
pub struct OsrRequestContextHandler {
    pub adblock_engine: Option<AdblockEngineHandle>,
}

impl OsrRequestContextHandler {
    pub fn new(adblock_engine: Option<AdblockEngineHandle>) -> Self {
        Self { adblock_engine }
    }
}

#[derive(Clone)]
pub struct OsrResourceRequestHandler {
    adblock_engine: Option<AdblockEngineHandle>,
}

wrap_resource_request_handler! {
    pub(crate) struct ResourceRequestHandlerImpl {
        handler: OsrResourceRequestHandler,
    }

    impl ResourceRequestHandler {
        fn on_before_resource_load(
            &self,
            _browser: Option<&mut cef::Browser>,
            _frame: Option<&mut cef::Frame>,
            request: Option<&mut cef::Request>,
            _callback: Option<&mut cef::Callback>,
        ) -> ReturnValue {
            if let Some(adblock_engine) = &self.handler.adblock_engine
                && let Some(request) = request
                && let Ok(request) = cef_request_to_adblock_request(request)
                && adblock_engine.check_network_request(&request).matched
            {
                return ReturnValue::CANCEL;
            }

            ReturnValue::CONTINUE
        }
    }
}

impl ResourceRequestHandlerImpl {
    pub(crate) fn build(handler: OsrResourceRequestHandler) -> cef::ResourceRequestHandler {
        Self::new(handler)
    }
}

wrap_request_context_handler! {
    pub(crate) struct RequestContextHandlerImpl {
        handler: OsrRequestContextHandler,
    }

    impl RequestContextHandler {
        fn resource_request_handler(
            &self,
            _browser: Option<&mut cef::Browser>,
            _frame: Option<&mut cef::Frame>,
            _request: Option<&mut cef::Request>,
            _is_navigation: ::std::os::raw::c_int,
            _is_download: ::std::os::raw::c_int,
            _request_initiator: Option<&cef::CefString>,
            _disable_default_handling: Option<&mut ::std::os::raw::c_int>,
        ) -> Option<cef::ResourceRequestHandler> {
            Some(ResourceRequestHandlerImpl::build(OsrResourceRequestHandler {
                adblock_engine: self.handler.adblock_engine.clone(),
            }))
        }
    }
}

impl RequestContextHandlerImpl {
    pub(crate) fn build(handler: OsrRequestContextHandler) -> cef::RequestContextHandler {
        Self::new(handler)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bgra_to_rgba_single_pixel() {
        // BGRA: B=10, G=20, R=30, A=255 → RGBA: R=30, G=20, B=10, A=255
        let bgra = vec![10, 20, 30, 255];
        let rgba = bgra_to_rgba(&bgra);
        assert_eq!(rgba, vec![30, 20, 10, 255]);
    }

    #[test]
    fn test_bgra_to_rgba_four_pixels_simd_path() {
        // Exactly 16 bytes (4 pixels) → triggers the SIMD path
        let bgra = vec![
            10, 20, 30, 255, // pixel 0: B=10, G=20, R=30
            0, 128, 255, 200, // pixel 1: B=0, G=128, R=255
            50, 50, 50, 128, // pixel 2: B=50, G=50, R=50
            255, 0, 0, 0, // pixel 3: B=255, G=0, R=0
        ];
        let rgba = bgra_to_rgba(&bgra);
        assert_eq!(
            rgba,
            vec![
                30, 20, 10, 255, // pixel 0: R=30, G=20, B=10
                255, 128, 0, 200, // pixel 1: R=255, G=128, B=0
                50, 50, 50, 128, // pixel 2: R=50, G=50, B=50
                0, 0, 255, 0, // pixel 3: R=0, G=0, B=255
            ]
        );
    }

    #[test]
    fn test_bgra_to_rgba_mixed_simd_and_remainder() {
        // 20 bytes = 4 pixels (SIMD) + 1 pixel (remainder)
        let bgra = vec![
            10, 20, 30, 255, 0, 128, 255, 200, 50, 50, 50, 128, 255, 0, 0, 0,
            // remainder pixel:
            100, 150, 200, 250,
        ];
        let rgba = bgra_to_rgba(&bgra);
        assert_eq!(rgba.len(), 20);
        // Check remainder pixel: BGRA(100,150,200,250) → RGBA(200,150,100,250)
        assert_eq!(&rgba[16..20], &[200, 150, 100, 250]);
    }

    #[test]
    fn test_bgra_to_rgba_empty() {
        let rgba = bgra_to_rgba(&[]);
        assert!(rgba.is_empty());
    }

    #[test]
    fn test_bgra_to_rgba_roundtrip() {
        // Converting BGRA→RGBA and then treating the result as BGRA and converting again
        // should yield the original data (since swapping R↔B is its own inverse).
        let original = vec![10, 20, 30, 255, 40, 50, 60, 128];
        let converted = bgra_to_rgba(&original);
        let roundtrip = bgra_to_rgba(&converted);
        assert_eq!(roundtrip, original);
    }
}
