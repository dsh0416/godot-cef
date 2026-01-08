use cef::{self, rc::Rc, sys::cef_cursor_type_t, *};
use cef_app::CursorType;
use std::sync::{Arc, Mutex};
use wide::{i8x16, u8x16};
use winit::dpi::PhysicalSize;

use crate::accelerated_osr::PlatformAcceleratedRenderHandler;
use crate::browser::MessageQueue;
use crate::utils::get_display_scale_factor;

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

wrap_render_handler! {
    pub struct SoftwareOsrHandler {
        handler: cef_app::OsrRenderHandler,
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
            _view_x: ::std::os::raw::c_int,
            _view_y: ::std::os::raw::c_int,
            _screen_x: Option<&mut ::std::os::raw::c_int>,
            _screen_y: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            false as _
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
    }
}

impl SoftwareOsrHandler {
    pub fn build(handler: cef_app::OsrRenderHandler) -> cef::RenderHandler {
        Self::new(handler)
    }
}

wrap_render_handler! {
    pub struct AcceleratedOsrHandler {
        handler: PlatformAcceleratedRenderHandler,
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
            _view_x: ::std::os::raw::c_int,
            _view_y: ::std::os::raw::c_int,
            _screen_x: Option<&mut ::std::os::raw::c_int>,
            _screen_y: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            false as _
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
    }
}

impl AcceleratedOsrHandler {
    pub fn build(handler: PlatformAcceleratedRenderHandler) -> cef::RenderHandler {
        Self::new(handler)
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
    }
}

impl DisplayHandlerImpl {
    pub fn build(cursor_type: Arc<Mutex<CursorType>>) -> cef::DisplayHandler {
        Self::new(cursor_type)
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

fn on_process_message_received(
    _browser: Option<&mut cef::Browser>,
    _frame: Option<&mut cef::Frame>,
    _source_process: ProcessId,
    message: Option<&mut ProcessMessage>,
    message_queue: &MessageQueue,
) -> i32 {
    let Some(message) = message else { return 0 };
    let route = CefStringUtf16::from(&message.name()).to_string();

    if route == "ipcRendererToGodot"
        && let Some(args) = message.argument_list()
    {
        let arg = args.string(0);
        let msg_str = CefStringUtf16::from(&arg).to_string();

        if let Ok(mut queue) = message_queue.lock() {
            queue.push_back(msg_str);
            return 1;
        }
    }

    0
}

wrap_client! {
    pub(crate) struct SoftwareClientImpl {
        render_handler: cef::RenderHandler,
        display_handler: cef::DisplayHandler,
        context_menu_handler: cef::ContextMenuHandler,
        life_span_handler: cef::LifeSpanHandler,
        message_queue: MessageQueue,
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

        fn on_process_message_received(
            &self,
            browser: Option<&mut cef::Browser>,
            frame: Option<&mut cef::Frame>,
            source_process: ProcessId,
            message: Option<&mut ProcessMessage>,
        ) -> i32 {
            on_process_message_received(browser, frame, source_process, message, &self.message_queue)
        }
    }
}

impl SoftwareClientImpl {
    pub(crate) fn build(
        render_handler: cef_app::OsrRenderHandler,
        message_queue: MessageQueue,
    ) -> cef::Client {
        let cursor_type = render_handler.get_cursor_type();
        Self::new(
            SoftwareOsrHandler::build(render_handler),
            DisplayHandlerImpl::build(cursor_type),
            ContextMenuHandlerImpl::build(),
            LifeSpanHandlerImpl::build(),
            message_queue,
        )
    }
}

wrap_client! {
    pub(crate) struct AcceleratedClientImpl {
        render_handler: cef::RenderHandler,
        display_handler: cef::DisplayHandler,
        context_menu_handler: cef::ContextMenuHandler,
        life_span_handler: cef::LifeSpanHandler,
        message_queue: MessageQueue,
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

        fn on_process_message_received(
            &self,
            browser: Option<&mut cef::Browser>,
            frame: Option<&mut cef::Frame>,
            source_process: ProcessId,
            message: Option<&mut ProcessMessage>,
        ) -> i32 {
            on_process_message_received(browser, frame, source_process, message, &self.message_queue)
        }
    }
}

impl AcceleratedClientImpl {
    pub(crate) fn build(
        render_handler: PlatformAcceleratedRenderHandler,
        cursor_type: Arc<Mutex<CursorType>>,
        message_queue: MessageQueue,
    ) -> cef::Client {
        Self::new(
            AcceleratedOsrHandler::build(render_handler),
            DisplayHandlerImpl::build(cursor_type),
            ContextMenuHandlerImpl::build(),
            LifeSpanHandlerImpl::build(),
            message_queue,
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
