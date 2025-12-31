mod cef_init;
mod cursor;
mod input;
mod utils;
mod webrender;

use cef::{
    quit_message_loop, run_message_loop, BrowserSettings, ImplBrowser, ImplBrowserHost,
    RequestContextSettings, WindowInfo,
};
use cef_app::{CursorType, FrameBuffer};
use godot::classes::image::Format as ImageFormat;
use godot::classes::notify::ControlNotification;
use godot::classes::texture_rect::ExpandMode;
use godot::classes::{
    ITextureRect, Image, ImageTexture, InputEvent, InputEventKey, InputEventMouseButton,
    InputEventMouseMotion, TextureRect,
};
use godot::init::*;
use godot::prelude::*;
use std::sync::{Arc, Mutex};
use winit::dpi::PhysicalSize;

use crate::cef_init::CEF_INITIALIZED;

struct GodotCef;

#[gdextension]
unsafe impl ExtensionLibrary for GodotCef {}

/// Internal application state for CEF browser
struct App {
    browser: Option<cef::Browser>,
    frame_buffer: Option<Arc<Mutex<FrameBuffer>>>,
    texture: Option<Gd<ImageTexture>>,
    render_size: Option<Arc<Mutex<PhysicalSize<f32>>>>,
    device_scale_factor: Option<Arc<Mutex<f32>>>,
    cursor_type: Option<Arc<Mutex<CursorType>>>,
    last_size: Vector2,
    last_dpi: f32,
    last_cursor: CursorType,
}

impl Default for App {
    fn default() -> Self {
        Self {
            browser: None,
            frame_buffer: None,
            texture: None,
            render_size: None,
            device_scale_factor: None,
            cursor_type: None,
            last_size: Vector2::ZERO,
            last_dpi: 1.0,
            last_cursor: CursorType::Arrow,
        }
    }
}

/// A Godot TextureRect that renders a CEF browser
#[derive(GodotClass)]
#[class(base=TextureRect)]
struct CefTexture {
    base: Base<TextureRect>,
    app: App,

    #[export]
    url: GString,

    #[export]
    enable_accelerated_osr: bool,
}

#[godot_api]
impl ITextureRect for CefTexture {
    fn init(base: Base<TextureRect>) -> Self {
        Self {
            base,
            app: App::default(),
            url: "https://google.com".into(),
            enable_accelerated_osr: true,
        }
    }

    fn ready(&mut self) {
        self.on_ready();
    }

    fn process(&mut self, _delta: f64) {
        self.on_process();
    }

    fn on_notification(&mut self, what: ControlNotification) {
        if let ControlNotification::WM_CLOSE_REQUEST = what {
            self.shutdown();
        }
    }

    fn input(&mut self, event: Gd<InputEvent>) {
        self.handle_input_event(event);
    }
}

#[godot_api]
impl CefTexture {
    // ========== Lifecycle ==========

    fn on_ready(&mut self) {
        self.base_mut().set_expand_mode(ExpandMode::IGNORE_SIZE);

        CEF_INITIALIZED.call_once(|| {
            cef_init::load_cef_framework();
            cef_init::initialize_cef();
        });

        self.create_browser();
        self.request_external_begin_frame();
    }

    fn on_process(&mut self) {
        self.handle_size_change();
        self.handle_dpi_change();

        if let Some(browser) = self.app.browser.as_mut() {
            if let Some(host) = browser.host() {
                host.send_external_begin_frame();
            }
        }

        run_message_loop();
        quit_message_loop();

        self.update_texture_from_buffer();
        self.update_cursor();
        self.request_external_begin_frame();
    }

    fn shutdown(&mut self) {
        self.app.browser = None;
        self.app.frame_buffer = None;
        self.app.texture = None;
        self.app.render_size = None;
        self.app.device_scale_factor = None;
        self.app.cursor_type = None;

        cef_init::shutdown_cef();
    }

    // ========== Browser Creation ==========

    fn create_browser(&mut self) {
        let logical_size = self.base().get_rect().size;
        let dpi = self.get_content_scale_factor();
        let pixel_width = (logical_size.x * dpi) as i32;
        let pixel_height = (logical_size.y * dpi) as i32;

        let window_info = WindowInfo {
            bounds: cef::Rect {
                x: 0,
                y: 0,
                width: pixel_width,
                height: pixel_height,
            },
            windowless_rendering_enabled: true as _,
            shared_texture_enabled: false as _,
            external_begin_frame_enabled: true as _,
            ..Default::default()
        };

        let browser_settings = BrowserSettings::default();

        let mut context = cef::request_context_create_context(
            Some(&RequestContextSettings::default()),
            Some(&mut webrender::RequestContextHandlerBuilder::build(
                webrender::OsrRequestContextHandler {},
            )),
        );

        let render_handler = cef_app::OsrRenderHandler::new(
            dpi,
            PhysicalSize::new(pixel_width as f32, pixel_height as f32),
        );
        self.create_texture_and_buffer(&render_handler, dpi);

        let mut client = webrender::ClientBuilder::build(render_handler);

        let browser = cef::browser_host_create_browser_sync(
            Some(&window_info),
            Some(&mut client),
            Some(&self.url.to_string().as_str().into()),
            Some(&browser_settings),
            None,
            context.as_mut(),
        );

        assert!(browser.is_some(), "failed to create browser");
        self.app.browser = browser;
    }

    fn create_texture_and_buffer(
        &mut self,
        render_handler: &cef_app::OsrRenderHandler,
        initial_dpi: f32,
    ) {
        let frame_buffer = render_handler.get_frame_buffer();
        let render_size = render_handler.get_size();
        let device_scale_factor = render_handler.get_device_scale_factor();
        let cursor_type = render_handler.get_cursor_type();

        let texture = ImageTexture::new_gd();
        self.base_mut().set_texture(&texture);

        self.app.frame_buffer = Some(frame_buffer);
        self.app.texture = Some(texture);
        self.app.render_size = Some(render_size);
        self.app.device_scale_factor = Some(device_scale_factor);
        self.app.cursor_type = Some(cursor_type);
        self.app.last_size = self.base().get_rect().size;
        self.app.last_dpi = initial_dpi;
    }

    // ========== Display Handling ==========

    fn get_content_scale_factor(&self) -> f32 {
        if let Some(tree) = self.base().get_tree() {
            if let Some(window) = tree.get_root() {
                return window.get_content_scale_factor();
            }
        }
        1.0
    }

    fn handle_dpi_change(&mut self) {
        let current_dpi = self.get_content_scale_factor();
        if (current_dpi - self.app.last_dpi).abs() < 0.01 {
            return;
        }

        if let Some(device_scale_factor) = &self.app.device_scale_factor {
            if let Ok(mut dpi) = device_scale_factor.lock() {
                *dpi = current_dpi;
            }
        }

        // DPI change means physical pixel count changed, update render size
        let logical_size = self.base().get_rect().size;
        let pixel_width = logical_size.x * current_dpi;
        let pixel_height = logical_size.y * current_dpi;

        if let Some(render_size) = &self.app.render_size {
            if let Ok(mut size) = render_size.lock() {
                size.width = pixel_width;
                size.height = pixel_height;
            }
        }

        if let Some(browser) = self.app.browser.as_mut() {
            if let Some(host) = browser.host() {
                host.notify_screen_info_changed();
                host.was_resized();
            }
        }

        self.app.last_dpi = current_dpi;
    }

    fn handle_size_change(&mut self) {
        let logical_size = self.base().get_rect().size;
        if logical_size.x <= 0.0 || logical_size.y <= 0.0 {
            return;
        }

        // 1px tolerance to avoid resize loops
        let size_diff = (logical_size - self.app.last_size).abs();
        if size_diff.x < 1.0 && size_diff.y < 1.0 {
            return;
        }

        let dpi = self.get_content_scale_factor();
        let pixel_width = logical_size.x * dpi;
        let pixel_height = logical_size.y * dpi;

        if let Some(render_size) = &self.app.render_size {
            if let Ok(mut size) = render_size.lock() {
                size.width = pixel_width;
                size.height = pixel_height;
            }
        }

        if let Some(browser) = self.app.browser.as_mut() {
            if let Some(host) = browser.host() {
                host.was_resized();
            }
        }

        self.app.last_size = logical_size;
    }

    // ========== Rendering ==========

    fn update_texture_from_buffer(&mut self) {
        let Some(frame_buffer_arc) = &self.app.frame_buffer else {
            return;
        };
        let Some(texture) = &mut self.app.texture else {
            return;
        };
        let Ok(mut frame_buffer) = frame_buffer_arc.lock() else {
            return;
        };
        if !frame_buffer.dirty || frame_buffer.data.is_empty() {
            return;
        }

        let width = frame_buffer.width as i32;
        let height = frame_buffer.height as i32;
        let byte_array = PackedByteArray::from(frame_buffer.data.as_slice());

        let image = Image::create_from_data(width, height, false, ImageFormat::RGBA8, &byte_array);
        if let Some(image) = image {
            texture.set_image(&image);
        }

        frame_buffer.mark_clean();
    }

    fn request_external_begin_frame(&mut self) {
        if let Some(browser) = self.app.browser.as_mut() {
            if let Some(host) = browser.host() {
                host.send_external_begin_frame();
            }
        }
    }

    // ========== Cursor ==========

    fn update_cursor(&mut self) {
        let cursor_type_arc = match &self.app.cursor_type {
            Some(arc) => arc.clone(),
            None => return,
        };

        let current_cursor = match cursor_type_arc.lock() {
            Ok(cursor_type) => *cursor_type,
            Err(_) => return,
        };

        if current_cursor == self.app.last_cursor {
            return;
        }

        self.app.last_cursor = current_cursor;
        let shape = cursor::cursor_type_to_shape(current_cursor);
        self.base_mut().set_default_cursor_shape(shape);
    }

    // ========== Input ==========

    fn handle_input_event(&mut self, event: Gd<InputEvent>) {
        let Some(browser) = self.app.browser.as_mut() else {
            return;
        };
        let Some(host) = browser.host() else {
            return;
        };

        let dpi = self.get_content_scale_factor();

        if let Ok(mouse_button) = event.clone().try_cast::<InputEventMouseButton>() {
            input::handle_mouse_button(&host, &mouse_button, dpi);
        } else if let Ok(mouse_motion) = event.clone().try_cast::<InputEventMouseMotion>() {
            input::handle_mouse_motion(&host, &mouse_motion, dpi);
        } else if let Ok(key_event) = event.try_cast::<InputEventKey>() {
            input::handle_key_event(&host, &key_event);
        }
    }

    // ========== IME Support ==========

    /// Commits IME text to the browser (call when IME composition is finalized)
    #[func]
    pub fn ime_commit_text(&mut self, text: GString) {
        let Some(browser) = self.app.browser.as_mut() else {
            return;
        };
        let Some(host) = browser.host() else {
            return;
        };
        input::ime_commit_text(&host, &text.to_string());
    }

    /// Sets the current IME composition text (call during IME composition)
    #[func]
    pub fn ime_set_composition(&mut self, text: GString) {
        let Some(browser) = self.app.browser.as_mut() else {
            return;
        };
        let Some(host) = browser.host() else {
            return;
        };
        input::ime_set_composition(&host, &text.to_string());
    }

    /// Cancels the current IME composition
    #[func]
    pub fn ime_cancel_composition(&mut self) {
        let Some(browser) = self.app.browser.as_mut() else {
            return;
        };
        let Some(host) = browser.host() else {
            return;
        };
        input::ime_cancel_composition(&host);
    }

    /// Finishes the current IME composition
    #[func]
    pub fn ime_finish_composing_text(&mut self, keep_selection: bool) {
        let Some(browser) = self.app.browser.as_mut() else {
            return;
        };
        let Some(host) = browser.host() else {
            return;
        };
        input::ime_finish_composing_text(&host, keep_selection);
    }
}
