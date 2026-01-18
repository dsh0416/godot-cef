use cef::{self, rc::Rc, sys::cef_cursor_type_t, *};
use cef_app::CursorType;
use std::sync::{Arc, Mutex};
use wide::{i8x16, u8x16};
use winit::dpi::PhysicalSize;

use crate::accelerated_osr::PlatformAcceleratedRenderHandler;
use crate::browser::{
    ConsoleMessageEvent, ConsoleMessageQueue, ImeCompositionQueue, ImeCompositionRange,
    ImeEnableQueue, LoadingStateEvent, LoadingStateQueue, MessageQueue, TitleChangeQueue,
    UrlChangeQueue,
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

wrap_render_handler! {
    pub struct SoftwareOsrHandler {
        handler: cef_app::OsrRenderHandler,
        ime_composition_queue: ImeCompositionQueue,
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

        fn on_paint(
            &self,
            _browser: Option<&mut Browser>,
            _type_: PaintElementType,
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

            if let Ok(mut frame_buffer) = self.handler.frame_buffer.lock() {
                frame_buffer.update(rgba_data, width, height);
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
    }
}

impl SoftwareOsrHandler {
    pub fn build(
        handler: cef_app::OsrRenderHandler,
        ime_composition_queue: ImeCompositionQueue,
    ) -> cef::RenderHandler {
        Self::new(handler, ime_composition_queue)
    }
}

wrap_render_handler! {
    pub struct AcceleratedOsrHandler {
        handler: PlatformAcceleratedRenderHandler,
        ime_composition_queue: ImeCompositionQueue,
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
            _type_: PaintElementType,
            _dirty_rects: Option<&[Rect]>,
            _buffer: *const u8,
            _width: ::std::os::raw::c_int,
            _height: ::std::os::raw::c_int,
        ) {
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
    }
}

impl AcceleratedOsrHandler {
    pub fn build(
        handler: PlatformAcceleratedRenderHandler,
        ime_composition_queue: ImeCompositionQueue,
    ) -> cef::RenderHandler {
        Self::new(handler, ime_composition_queue)
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
            // level.get_raw() returns a u32 on macOS and Linux,
            // but a i32 on Windows.
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

fn on_process_message_received(
    _browser: Option<&mut cef::Browser>,
    _frame: Option<&mut cef::Frame>,
    _source_process: ProcessId,
    message: Option<&mut ProcessMessage>,
    message_queue: &MessageQueue,
    ime_enable_queue: &ImeEnableQueue,
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
        _ => {}
    }

    0
}

wrap_client! {
    pub(crate) struct SoftwareClientImpl {
        render_handler: cef::RenderHandler,
        display_handler: cef::DisplayHandler,
        context_menu_handler: cef::ContextMenuHandler,
        life_span_handler: cef::LifeSpanHandler,
        load_handler: cef::LoadHandler,
        message_queue: MessageQueue,
        ime_enable_queue: ImeEnableQueue,
    }

    impl Client {
        fn render_handler(&self) -> Option<cef::RenderHandler> {
            Some(self.render_handler.clone())
        }

        fn display_handler(&self) -> Option<cef::DisplayHandler> {
            Some(self.display_handler.clone())
        }

        fn context_menu_handler(&self) -> Option<cef::ContextMenuHandler> {
            Some(self.context_menu_handler.clone())
        }

        fn life_span_handler(&self) -> Option<cef::LifeSpanHandler> {
            Some(self.life_span_handler.clone())
        }

        fn load_handler(&self) -> Option<cef::LoadHandler> {
            Some(self.load_handler.clone())
        }

        fn on_process_message_received(
            &self,
            browser: Option<&mut cef::Browser>,
            frame: Option<&mut cef::Frame>,
            source_process: ProcessId,
            message: Option<&mut ProcessMessage>,
        ) -> i32 {
            on_process_message_received(browser, frame, source_process, message, &self.message_queue, &self.ime_enable_queue)
        }
    }
}

fn build_client_common(
    cursor_type: Arc<Mutex<CursorType>>,
    url_change_queue: UrlChangeQueue,
    title_change_queue: TitleChangeQueue,
    loading_state_queue: LoadingStateQueue,
    console_message_queue: ConsoleMessageQueue,
) -> (
    cef::DisplayHandler,
    cef::ContextMenuHandler,
    cef::LifeSpanHandler,
    cef::LoadHandler,
) {
    (
        DisplayHandlerImpl::build(
            cursor_type,
            url_change_queue,
            title_change_queue,
            console_message_queue,
        ),
        ContextMenuHandlerImpl::build(),
        LifeSpanHandlerImpl::build(),
        LoadHandlerImpl::build(loading_state_queue),
    )
}

impl SoftwareClientImpl {
    pub(crate) fn build(
        render_handler: cef_app::OsrRenderHandler,
        queues: ClientQueues,
    ) -> cef::Client {
        let cursor_type = render_handler.get_cursor_type();
        let (display_handler, context_menu_handler, life_span_handler, load_handler) =
            build_client_common(
                cursor_type.clone(),
                queues.url_change_queue,
                queues.title_change_queue,
                queues.loading_state_queue,
                queues.console_message_queue,
            );
        Self::new(
            SoftwareOsrHandler::build(render_handler, queues.ime_composition_queue),
            display_handler,
            context_menu_handler,
            life_span_handler,
            load_handler,
            queues.message_queue,
            queues.ime_enable_queue,
        )
    }
}

wrap_client! {
    pub(crate) struct AcceleratedClientImpl {
        render_handler: cef::RenderHandler,
        display_handler: cef::DisplayHandler,
        context_menu_handler: cef::ContextMenuHandler,
        life_span_handler: cef::LifeSpanHandler,
        load_handler: cef::LoadHandler,
        message_queue: MessageQueue,
        ime_enable_queue: ImeEnableQueue,
    }

    impl Client {
        fn render_handler(&self) -> Option<cef::RenderHandler> {
            Some(self.render_handler.clone())
        }

        fn display_handler(&self) -> Option<cef::DisplayHandler> {
            Some(self.display_handler.clone())
        }

        fn context_menu_handler(&self) -> Option<cef::ContextMenuHandler> {
            Some(self.context_menu_handler.clone())
        }

        fn life_span_handler(&self) -> Option<cef::LifeSpanHandler> {
            Some(self.life_span_handler.clone())
        }

        fn load_handler(&self) -> Option<cef::LoadHandler> {
            Some(self.load_handler.clone())
        }

        fn on_process_message_received(
            &self,
            browser: Option<&mut cef::Browser>,
            frame: Option<&mut cef::Frame>,
            source_process: ProcessId,
            message: Option<&mut ProcessMessage>,
        ) -> i32 {
            on_process_message_received(browser, frame, source_process, message, &self.message_queue, &self.ime_enable_queue)
        }
    }
}

impl AcceleratedClientImpl {
    pub(crate) fn build(
        render_handler: PlatformAcceleratedRenderHandler,
        cursor_type: Arc<Mutex<CursorType>>,
        queues: ClientQueues,
    ) -> cef::Client {
        let (display_handler, context_menu_handler, life_span_handler, load_handler) =
            build_client_common(
                cursor_type.clone(),
                queues.url_change_queue,
                queues.title_change_queue,
                queues.loading_state_queue,
                queues.console_message_queue,
            );
        Self::new(
            AcceleratedOsrHandler::build(render_handler, queues.ime_composition_queue),
            display_handler,
            context_menu_handler,
            life_span_handler,
            load_handler,
            queues.message_queue,
            queues.ime_enable_queue,
        )
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
