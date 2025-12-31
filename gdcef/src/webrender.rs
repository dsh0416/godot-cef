use cef::{self, rc::Rc, *,};
use godot::global::godot_print;

/// Convert BGRA pixel data to RGBA by swapping B and R channels
fn bgra_to_rgba(bgra: &[u8]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(bgra.len());
    for chunk in bgra.chunks_exact(4) {
        rgba.push(chunk[2]); // R (from B)
        rgba.push(chunk[1]); // G
        rgba.push(chunk[0]); // B (from R)
        rgba.push(chunk[3]); // A
    }
    rgba
}

wrap_render_handler! {
    pub struct RenderHandlerBuilder {
        handler: cef_app::OsrRenderHandler,
    }

    impl RenderHandler {
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
            if let Some(rect) = rect {
                if let Ok(size) = self.handler.size.lock() {
                    // size must be non-zero
                    if size.width > 0.0 && size.height > 0.0 {
                        rect.width = size.width as _;
                        rect.height = size.height as _;
                    }
                }
            }
        }

        fn screen_info(
            &self,
            _browser: Option<&mut Browser>,
            screen_info: Option<&mut ScreenInfo>,
        ) -> ::std::os::raw::c_int {
            if let Some(screen_info) = screen_info {
                screen_info.device_scale_factor = self.handler.device_scale_factor;
                return true as _;
            }
            false as _
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
            _info: Option<&AcceleratedPaintInfo>,
        ) {
            godot_print!("on_accelerated_paint, type: {:?}", type_);
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

            // Safety: CEF guarantees the buffer is valid for width * height * 4 bytes
            let bgra_data = unsafe { std::slice::from_raw_parts(buffer, buffer_size) };

            // Convert BGRA to RGBA
            let rgba_data = bgra_to_rgba(bgra_data);

            // Store in the shared frame buffer
            if let Ok(mut frame_buffer) = self.handler.frame_buffer.lock() {
                frame_buffer.update(rgba_data, width, height);
            }
        }
    }
}

impl RenderHandlerBuilder {
    pub fn build(handler: cef_app::OsrRenderHandler) -> RenderHandler {
        Self::new(handler)
    }
}

wrap_client! {
    pub(crate) struct ClientBuilder {
        render_handler: RenderHandler,
    }

    impl Client {
        fn render_handler(&self) -> Option<cef::RenderHandler> {
            Some(self.render_handler.clone())
        }
    }
}

impl ClientBuilder {
    pub(crate) fn build(render_handler: cef_app::OsrRenderHandler) -> Client {
        Self::new(RenderHandlerBuilder::build(render_handler))
    }
}

#[derive(Clone)]
pub struct OsrRequestContextHandler {}

wrap_request_context_handler! {
    pub(crate) struct RequestContextHandlerBuilder {
        handler: OsrRequestContextHandler,
    }

    impl RequestContextHandler {}
}

impl RequestContextHandlerBuilder {
    pub(crate) fn build(handler: OsrRequestContextHandler) -> RequestContextHandler {
        Self::new(handler)
    }
}
