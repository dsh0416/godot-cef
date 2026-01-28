use cef::{self, rc::Rc, sys::cef_cursor_type_t, *};
use cef_app::{CursorType, PhysicalSize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use wide::{i8x16, u8x16};

use crate::accelerated_osr::PlatformAcceleratedRenderHandler;
use crate::browser::{
    AudioPacket, AudioPacketQueue, AudioParamsState, AudioSampleRateState, ConsoleMessageEvent,
    ConsoleMessageQueue, DragDataInfo, DragEvent, DragEventQueue, ImeCompositionQueue,
    ImeCompositionRange, ImeEnableQueue, LoadingStateEvent, LoadingStateQueue, MessageQueue,
    TitleChangeQueue, UrlChangeQueue,
};
use crate::utils::get_display_scale_factor;

/// Bundles all the event queues used for browser-to-Godot communication.
pub(crate) struct ClientQueues {
    pub message_queue: MessageQueue,
    pub url_change_queue: UrlChangeQueue,
    pub title_change_queue: TitleChangeQueue,
    pub loading_state_queue: LoadingStateQueue,
    pub ime_enable_queue: ImeEnableQueue,
    pub ime_composition_queue: ImeCompositionQueue,
    pub console_message_queue: ConsoleMessageQueue,
    pub drag_event_queue: DragEventQueue,
    pub audio_packet_queue: AudioPacketQueue,
    pub audio_params: AudioParamsState,
    pub audio_sample_rate: AudioSampleRateState,
    pub enable_audio_capture: bool,
}

impl ClientQueues {
    pub fn new(sample_rate: i32, enable_audio_capture: bool) -> Self {
        Self {
            message_queue: Arc::new(Mutex::new(VecDeque::new())),
            url_change_queue: Arc::new(Mutex::new(VecDeque::new())),
            title_change_queue: Arc::new(Mutex::new(VecDeque::new())),
            loading_state_queue: Arc::new(Mutex::new(VecDeque::new())),
            ime_enable_queue: Arc::new(Mutex::new(VecDeque::new())),
            ime_composition_queue: Arc::new(Mutex::new(None)),
            console_message_queue: Arc::new(Mutex::new(VecDeque::new())),
            drag_event_queue: Arc::new(Mutex::new(VecDeque::new())),
            audio_packet_queue: Arc::new(Mutex::new(VecDeque::new())),
            audio_params: Arc::new(Mutex::new(None)),
            audio_sample_rate: Arc::new(Mutex::new(sample_rate)),
            enable_audio_capture,
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
        let src: [u8; 16] = bgra[offset..offset + 16].try_into().unwrap();
        let v = u8x16::new(src);
        // Swizzle BGRA -> RGBA using precomputed indices
        let shuffled = v.swizzle(BGRA_TO_RGBA_INDICES);
        let result: [i8; 16] = shuffled.into();
        // Safe transmute: i8 and u8 have identical bit representation
        let result_u8: [u8; 16] = unsafe { std::mem::transmute(result) };
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

/// Common helper for start_dragging implementation.
fn handle_start_dragging(
    drag_data: Option<&mut DragData>,
    allowed_ops: DragOperationsMask,
    x: ::std::os::raw::c_int,
    y: ::std::os::raw::c_int,
    drag_event_queue: &DragEventQueue,
) -> ::std::os::raw::c_int {
    if let Some(drag_data) = drag_data {
        let drag_info = extract_drag_data_info(drag_data);
        if let Ok(mut queue) = drag_event_queue.lock() {
            queue.push_back(DragEvent::Started {
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
fn handle_update_drag_cursor(operation: DragOperationsMask, drag_event_queue: &DragEventQueue) {
    if let Ok(mut queue) = drag_event_queue.lock() {
        queue.push_back(DragEvent::UpdateCursor {
            operation: drag_ops_to_u32(operation),
        });
    }
}

wrap_render_handler! {
    pub struct SoftwareOsrHandler {
        handler: cef_app::OsrRenderHandler,
        ime_composition_queue: ImeCompositionQueue,
        drag_event_queue: DragEventQueue,
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
                && let Ok(mut queue) = self.ime_composition_queue.lock() {
                    *queue = Some(ImeCompositionRange {
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
            handle_start_dragging(drag_data, allowed_ops, x, y, &self.drag_event_queue)
        }

        fn update_drag_cursor(
            &self,
            _browser: Option<&mut Browser>,
            operation: DragOperationsMask,
        ) {
            handle_update_drag_cursor(operation, &self.drag_event_queue);
        }
    }
}

impl SoftwareOsrHandler {
    pub fn build(
        handler: cef_app::OsrRenderHandler,
        ime_composition_queue: ImeCompositionQueue,
        drag_event_queue: DragEventQueue,
    ) -> cef::RenderHandler {
        Self::new(handler, ime_composition_queue, drag_event_queue)
    }
}

wrap_render_handler! {
    pub struct AcceleratedOsrHandler {
        handler: PlatformAcceleratedRenderHandler,
        ime_composition_queue: ImeCompositionQueue,
        drag_event_queue: DragEventQueue,
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
                && let Ok(mut queue) = self.ime_composition_queue.lock() {
                    *queue = Some(ImeCompositionRange {
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
            handle_start_dragging(drag_data, allowed_ops, x, y, &self.drag_event_queue)
        }

        fn update_drag_cursor(
            &self,
            _browser: Option<&mut Browser>,
            operation: DragOperationsMask,
        ) {
            handle_update_drag_cursor(operation, &self.drag_event_queue);
        }
    }
}

impl AcceleratedOsrHandler {
    pub fn build(
        handler: PlatformAcceleratedRenderHandler,
        ime_composition_queue: ImeCompositionQueue,
        drag_event_queue: DragEventQueue,
    ) -> cef::RenderHandler {
        Self::new(handler, ime_composition_queue, drag_event_queue)
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
        drag_event_queue: DragEventQueue,
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
                if let Ok(mut queue) = self.drag_event_queue.lock() {
                    #[cfg(target_os = "windows")]
                    let mask: u32 = mask.as_ref().0 as u32;
                    #[cfg(not(target_os = "windows"))]
                    let mask: u32 = mask.as_ref().0;

                    queue.push_back(DragEvent::Entered {
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
    pub fn build(drag_event_queue: DragEventQueue) -> cef::DragHandler {
        Self::new(drag_event_queue)
    }
}

wrap_display_handler! {
    pub(crate) struct DisplayHandlerImpl {
        cursor_type: Arc<Mutex<CursorType>>,
        url_change_queue: UrlChangeQueue,
        title_change_queue: TitleChangeQueue,
        console_message_queue: ConsoleMessageQueue,
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
                if let Ok(mut queue) = self.url_change_queue.lock() {
                    queue.push_back(url_str);
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
                if let Ok(mut queue) = self.title_change_queue.lock() {
                    queue.push_back(title_str);
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

            if let Ok(mut queue) = self.console_message_queue.lock() {
                queue.push_back(ConsoleMessageEvent {
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
        url_change_queue: UrlChangeQueue,
        title_change_queue: TitleChangeQueue,
        console_message_queue: ConsoleMessageQueue,
    ) -> cef::DisplayHandler {
        Self::new(
            cursor_type,
            url_change_queue,
            title_change_queue,
            console_message_queue,
        )
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
    pub(crate) struct LifeSpanHandlerImpl {}

    impl LifeSpanHandler {
        // Disable popup for now
        fn on_before_popup(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _popup_id: ::std::os::raw::c_int,
            _target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            _target_disposition: WindowOpenDisposition,
            _user_gesture: ::std::os::raw::c_int,
            _popup_features: Option<&PopupFeatures>,
            _window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            true as _
        }
    }
}

impl LifeSpanHandlerImpl {
    pub fn build() -> cef::LifeSpanHandler {
        Self::new()
    }
}

wrap_load_handler! {
    pub(crate) struct LoadHandlerImpl {
        loading_state_queue: LoadingStateQueue,
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
                if let Ok(mut queue) = self.loading_state_queue.lock() {
                    queue.push_back(LoadingStateEvent::Started { url });
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
                if let Ok(mut queue) = self.loading_state_queue.lock() {
                    queue.push_back(LoadingStateEvent::Finished {
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
                if let Ok(mut queue) = self.loading_state_queue.lock() {
                    queue.push_back(LoadingStateEvent::Error {
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
    pub fn build(loading_state_queue: LoadingStateQueue) -> cef::LoadHandler {
        Self::new(loading_state_queue)
    }
}

wrap_audio_handler! {
    pub(crate) struct AudioHandlerImpl {
        audio_params: AudioParamsState,
        audio_packet_queue: AudioPacketQueue,
        audio_sample_rate: AudioSampleRateState,
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
                    .unwrap_or(48000);

                params.channel_layout = ChannelLayout::LAYOUT_STEREO;
                params.sample_rate = sample_rate;
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
    ) -> cef::AudioHandler {
        Self::new(audio_params, audio_packet_queue, audio_sample_rate)
    }
}

fn on_process_message_received(
    _browser: Option<&mut cef::Browser>,
    _frame: Option<&mut cef::Frame>,
    _source_process: ProcessId,
    message: Option<&mut ProcessMessage>,
    message_queue: &MessageQueue,
    ime_enable_queue: &ImeEnableQueue,
    ime_composition_queue: &ImeCompositionQueue,
) -> i32 {
    let Some(message) = message else { return 0 };
    let route = CefStringUtf16::from(&message.name()).to_string();

    match route.as_str() {
        "ipcRendererToGodot" => {
            if let Some(args) = message.argument_list() {
                let arg = args.string(0);
                let msg_str = CefStringUtf16::from(&arg).to_string();
                if let Ok(mut queue) = message_queue.lock() {
                    queue.push_back(msg_str);
                }
            }
        }
        "triggerIme" => {
            if let Some(args) = message.argument_list() {
                let arg = args.bool(0);
                let enabled = arg != 0;
                if let Ok(mut queue) = ime_enable_queue.lock() {
                    queue.push_back(enabled);
                }
            }
        }
        "imeCaretPosition" => {
            if let Some(args) = message.argument_list() {
                let x = args.int(0);
                let y = args.int(1);
                let height = args.int(2);
                if let Ok(mut queue) = ime_composition_queue.lock() {
                    *queue = Some(ImeCompositionRange {
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
}

#[derive(Clone)]
pub(crate) struct ClientIpcQueues {
    pub message_queue: MessageQueue,
    pub ime_enable_queue: ImeEnableQueue,
    pub ime_composition_queue: ImeCompositionQueue,
}

fn build_ipc_queues(queues: &ClientQueues) -> ClientIpcQueues {
    ClientIpcQueues {
        message_queue: queues.message_queue.clone(),
        ime_enable_queue: queues.ime_enable_queue.clone(),
        ime_composition_queue: queues.ime_composition_queue.clone(),
    }
}

wrap_client! {
    pub(crate) struct SoftwareClientImpl {
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

        fn on_process_message_received(
            &self,
            browser: Option<&mut cef::Browser>,
            frame: Option<&mut cef::Frame>,
            source_process: ProcessId,
            message: Option<&mut ProcessMessage>,
        ) -> i32 {
            on_process_message_received(browser, frame, source_process, message, &self.ipc.message_queue, &self.ipc.ime_enable_queue, &self.ipc.ime_composition_queue)
        }
    }
}

fn build_client_handlers(
    render_handler: cef::RenderHandler,
    cursor_type: Arc<Mutex<CursorType>>,
    queues: &ClientQueues,
) -> ClientHandlers {
    let audio_handler = if queues.enable_audio_capture {
        Some(AudioHandlerImpl::build(
            queues.audio_params.clone(),
            queues.audio_packet_queue.clone(),
            queues.audio_sample_rate.clone(),
        ))
    } else {
        None
    };

    ClientHandlers {
        render_handler,
        display_handler: DisplayHandlerImpl::build(
            cursor_type,
            queues.url_change_queue.clone(),
            queues.title_change_queue.clone(),
            queues.console_message_queue.clone(),
        ),
        context_menu_handler: ContextMenuHandlerImpl::build(),
        life_span_handler: LifeSpanHandlerImpl::build(),
        load_handler: LoadHandlerImpl::build(queues.loading_state_queue.clone()),
        drag_handler: DragHandlerImpl::build(queues.drag_event_queue.clone()),
        audio_handler,
    }
}

impl SoftwareClientImpl {
    pub(crate) fn build(
        render_handler: cef_app::OsrRenderHandler,
        queues: ClientQueues,
    ) -> cef::Client {
        let cursor_type = render_handler.get_cursor_type();
        let ipc = build_ipc_queues(&queues);
        let handlers = build_client_handlers(
            SoftwareOsrHandler::build(
                render_handler,
                queues.ime_composition_queue.clone(),
                queues.drag_event_queue.clone(),
            ),
            cursor_type,
            &queues,
        );
        Self::new(handlers, ipc)
    }
}

wrap_client! {
    pub(crate) struct AcceleratedClientImpl {
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

        fn on_process_message_received(
            &self,
            browser: Option<&mut cef::Browser>,
            frame: Option<&mut cef::Frame>,
            source_process: ProcessId,
            message: Option<&mut ProcessMessage>,
        ) -> i32 {
            on_process_message_received(browser, frame, source_process, message, &self.ipc.message_queue, &self.ipc.ime_enable_queue, &self.ipc.ime_composition_queue)
        }
    }
}

impl AcceleratedClientImpl {
    pub(crate) fn build(
        render_handler: PlatformAcceleratedRenderHandler,
        cursor_type: Arc<Mutex<CursorType>>,
        queues: ClientQueues,
    ) -> cef::Client {
        let ipc = build_ipc_queues(&queues);
        let handlers = build_client_handlers(
            AcceleratedOsrHandler::build(
                render_handler,
                queues.ime_composition_queue.clone(),
                queues.drag_event_queue.clone(),
            ),
            cursor_type,
            &queues,
        );
        Self::new(handlers, ipc)
    }
}

#[derive(Clone)]
pub struct OsrRequestContextHandler {}

wrap_request_context_handler! {
    pub(crate) struct RequestContextHandlerImpl {
        handler: OsrRequestContextHandler,
    }

    impl RequestContextHandler {}
}

impl RequestContextHandlerImpl {
    pub(crate) fn build(handler: OsrRequestContextHandler) -> cef::RequestContextHandler {
        Self::new(handler)
    }
}
