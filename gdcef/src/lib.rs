mod accelerated_osr;
mod browser;
mod cef_init;
mod cursor;
mod error;
mod input;
mod render;
mod res_protocol;
mod texture;
mod utils;
mod webrender;

use cef::{
    BrowserSettings, ImplBrowser, ImplBrowserHost, ImplFrame, RequestContextSettings, WindowInfo,
    api_hash, do_message_loop_work,
};
use godot::classes::image::Format as ImageFormat;
use godot::classes::notify::ControlNotification;
use godot::classes::texture_rect::ExpandMode;
use godot::classes::{
    DisplayServer, Engine, ITextureRect, Image, ImageTexture, InputEvent, InputEventKey,
    InputEventMouseButton, InputEventMouseMotion, InputEventPanGesture, TextureRect,
};
use godot::init::*;
use godot::prelude::*;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use winit::dpi::PhysicalSize;

use crate::accelerated_osr::{
    GodotTextureImporter, NativeHandleTrait, PlatformAcceleratedRenderHandler, TextureImporterTrait,
};
use crate::browser::{App, MessageQueue, RenderMode, UrlChangeQueue};
use crate::cef_init::CEF_INITIALIZED;

pub use texture::TextureRectRd;

struct GodotCef;

#[gdextension]
unsafe impl ExtensionLibrary for GodotCef {}

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

    fn on_notification(&mut self, what: ControlNotification) {
        match what {
            ControlNotification::READY => {
                self.on_ready();
            }
            ControlNotification::PROCESS => {
                self.on_process();
            }
            ControlNotification::WM_CLOSE_REQUEST => {
                self.shutdown();
            }
            ControlNotification::FOCUS_ENTER => {
                self.on_focus_enter();
            }
            ControlNotification::FOCUS_EXIT => {
                self.on_focus_exit();
            }
            _ => {}
        }
    }

    fn input(&mut self, event: Gd<InputEvent>) {
        self.handle_input_event(event);
    }
}

#[godot_api]
impl CefTexture {
    #[signal]
    fn ipc_message(message: GString);

    #[signal]
    fn url_changed(url: GString);

    #[func]
    fn on_ready(&mut self) {
        self.base_mut().set_expand_mode(ExpandMode::IGNORE_SIZE);

        CEF_INITIALIZED.call_once(|| {
            cef_init::load_cef_framework();
            api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
            cef_init::initialize_cef();
        });

        self.create_browser();
    }

    #[func]
    fn on_process(&mut self) {
        self.handle_max_fps_change();
        _ = self.handle_size_change();
        self.update_texture();

        do_message_loop_work();

        self.request_external_begin_frame();
        self.update_cursor();
        self.process_message_queue();
        self.process_url_change_queue();
    }

    fn shutdown(&mut self) {
        // Hide the TextureRect and clear its texture BEFORE freeing resources.
        // This prevents Godot from trying to render with an invalid texture during shutdown.
        self.base_mut().set_visible(false);

        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        if let Some(RenderMode::Accelerated {
            rd_texture_rid,
            texture_2d_rd,
            ..
        }) = &mut self.app.render_mode
        {
            // Clear the RD texture RID from the Texture2Drd to break the reference
            // before we free the underlying RD texture.
            texture_2d_rd.set_texture_rd_rid(Rid::Invalid);
            render::free_rd_texture(*rd_texture_rid);
        }

        self.app.browser = None;
        self.app.render_mode = None;
        self.app.render_size = None;
        self.app.device_scale_factor = None;
        self.app.cursor_type = None;
        self.app.message_queue = None;
        self.app.url_change_queue = None;
        self.app.last_max_fps = self.get_max_fps();

        cef_init::shutdown_cef();
    }

    fn create_browser(&mut self) {
        let logical_size = self.base().get_size();
        let dpi = self.get_pixel_scale_factor();
        let pixel_width = (logical_size.x * dpi) as i32;
        let pixel_height = (logical_size.y * dpi) as i32;

        let use_accelerated = self.should_use_accelerated_osr();

        let window_info = WindowInfo {
            bounds: cef::Rect {
                x: 0,
                y: 0,
                width: pixel_width,
                height: pixel_height,
            },
            windowless_rendering_enabled: true as _,
            shared_texture_enabled: use_accelerated as _,
            external_begin_frame_enabled: true as _,
            ..Default::default()
        };

        let browser_settings = BrowserSettings {
            windowless_frame_rate: self.get_max_fps(),
            ..Default::default()
        };

        let mut context = cef::request_context_create_context(
            Some(&RequestContextSettings::default()),
            Some(&mut webrender::RequestContextHandlerImpl::build(
                webrender::OsrRequestContextHandler {},
            )),
        );

        // Register the res:// scheme handler on this specific request context
        if let Some(ctx) = context.as_mut() {
            res_protocol::register_res_scheme_handler_on_context(ctx);
        }

        let browser = if use_accelerated {
            self.create_accelerated_browser(
                &window_info,
                &browser_settings,
                context.as_mut(),
                dpi,
                pixel_width,
                pixel_height,
            )
        } else {
            self.create_software_browser(
                &window_info,
                &browser_settings,
                context.as_mut(),
                dpi,
                pixel_width,
                pixel_height,
            )
        };

        assert!(browser.is_some(), "failed to create browser");
        self.app.browser = browser;
        self.app.last_size = logical_size;
        self.app.last_dpi = dpi;
    }

    fn should_use_accelerated_osr(&self) -> bool {
        self.enable_accelerated_osr && accelerated_osr::is_accelerated_osr_supported()
    }

    fn create_software_browser(
        &mut self,
        _window_info: &WindowInfo,
        browser_settings: &BrowserSettings,
        context: Option<&mut cef::RequestContext>,
        dpi: f32,
        pixel_width: i32,
        pixel_height: i32,
    ) -> Option<cef::Browser> {
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

        let render_handler = cef_app::OsrRenderHandler::new(
            dpi,
            PhysicalSize::new(pixel_width as f32, pixel_height as f32),
        );

        let frame_buffer = render_handler.get_frame_buffer();
        let render_size = render_handler.get_size();
        let device_scale_factor = render_handler.get_device_scale_factor();
        let cursor_type = render_handler.get_cursor_type();
        let message_queue: MessageQueue = Arc::new(Mutex::new(VecDeque::new()));
        let url_change_queue: UrlChangeQueue = Arc::new(Mutex::new(VecDeque::new()));

        let texture = ImageTexture::new_gd();
        self.base_mut().set_texture(&texture);

        self.app.render_mode = Some(RenderMode::Software {
            frame_buffer,
            texture,
        });
        self.app.render_size = Some(render_size);
        self.app.device_scale_factor = Some(device_scale_factor);
        self.app.cursor_type = Some(cursor_type);
        self.app.message_queue = Some(message_queue.clone());
        self.app.url_change_queue = Some(url_change_queue.clone());

        let mut client =
            webrender::SoftwareClientImpl::build(render_handler, message_queue, url_change_queue);

        cef::browser_host_create_browser_sync(
            Some(&window_info),
            Some(&mut client),
            Some(&self.url.to_string().as_str().into()),
            Some(browser_settings),
            None,
            context,
        )
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn create_accelerated_browser(
        &mut self,
        window_info: &WindowInfo,
        browser_settings: &BrowserSettings,
        context: Option<&mut cef::RequestContext>,
        dpi: f32,
        pixel_width: i32,
        pixel_height: i32,
    ) -> Option<cef::Browser> {
        let importer = match GodotTextureImporter::new() {
            Some(imp) => imp,
            None => {
                godot::global::godot_warn!(
                    "Failed to create GPU texture importer, falling back to software rendering"
                );
                return self.create_software_browser(
                    window_info,
                    browser_settings,
                    context,
                    dpi,
                    pixel_width,
                    pixel_height,
                );
            }
        };

        let render_handler = PlatformAcceleratedRenderHandler::new(
            dpi,
            PhysicalSize::new(pixel_width as f32, pixel_height as f32),
        );

        let texture_info = render_handler.get_texture_info();
        let render_size = render_handler.get_size();
        let device_scale_factor = render_handler.get_device_scale_factor();
        let cursor_type = render_handler.get_cursor_type();
        let message_queue: MessageQueue = Arc::new(Mutex::new(VecDeque::new()));
        let url_change_queue: UrlChangeQueue = Arc::new(Mutex::new(VecDeque::new()));

        let (rd_texture_rid, texture_2d_rd) = render::create_rd_texture(pixel_width, pixel_height);
        self.base_mut().set_texture(&texture_2d_rd);

        self.app.render_mode = Some(RenderMode::Accelerated {
            texture_info,
            importer,
            rd_texture_rid,
            texture_2d_rd,
            texture_width: pixel_width as u32,
            texture_height: pixel_height as u32,
        });
        self.app.render_size = Some(render_size);
        self.app.device_scale_factor = Some(device_scale_factor);
        self.app.cursor_type = Some(cursor_type);
        self.app.message_queue = Some(message_queue.clone());
        self.app.url_change_queue = Some(url_change_queue.clone());

        let mut client = webrender::AcceleratedClientImpl::build(
            render_handler,
            self.app.cursor_type.clone().unwrap(),
            message_queue,
            url_change_queue,
        );

        cef::browser_host_create_browser_sync(
            Some(window_info),
            Some(&mut client),
            Some(&self.url.to_string().as_str().into()),
            Some(browser_settings),
            None,
            context,
        )
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    fn create_accelerated_browser(
        &mut self,
        window_info: &WindowInfo,
        browser_settings: &BrowserSettings,
        context: Option<&mut cef::RequestContext>,
        dpi: f32,
        pixel_width: i32,
        pixel_height: i32,
    ) -> Option<cef::Browser> {
        self.create_software_browser(
            window_info,
            browser_settings,
            context,
            dpi,
            pixel_width,
            pixel_height,
        )
    }

    fn get_pixel_scale_factor(&self) -> f32 {
        self.base()
            .get_viewport()
            .unwrap()
            .get_stretch_transform()
            .a
            .x
    }

    fn get_device_scale_factor(&self) -> f32 {
        utils::get_display_scale_factor()
    }

    fn get_max_fps(&self) -> i32 {
        let engine_cap_fps = Engine::singleton().get_max_fps();
        let screen_cap_fps = DisplayServer::singleton().screen_get_refresh_rate().round() as i32;
        if engine_cap_fps > 0 {
            engine_cap_fps
        } else if screen_cap_fps > 0 {
            screen_cap_fps
        } else {
            60
        }
    }

    fn handle_max_fps_change(&mut self) {
        let max_fps = self.get_max_fps();
        if max_fps == self.app.last_max_fps {
            return;
        }

        self.app.last_max_fps = max_fps;
        if let Some(browser) = self.app.browser.as_mut()
            && let Some(host) = browser.host()
        {
            host.set_windowless_frame_rate(max_fps);
        }
    }

    fn handle_size_change(&mut self) -> bool {
        let current_dpi = self.get_pixel_scale_factor();
        let logical_size = self.base().get_size();
        if logical_size.x <= 0.0 || logical_size.y <= 0.0 {
            return false;
        }

        let size_diff = (logical_size - self.app.last_size).abs();
        let dpi_diff = (current_dpi - self.app.last_dpi).abs();
        if size_diff.x < 1e-6 && size_diff.y < 1e-6 && dpi_diff < 1e-6 {
            return false;
        }

        let pixel_width = logical_size.x * current_dpi;
        let pixel_height = logical_size.y * current_dpi;

        if let Some(render_size) = &self.app.render_size
            && let Ok(mut size) = render_size.lock()
        {
            size.width = pixel_width;
            size.height = pixel_height;
        }

        if let Some(device_scale_factor) = &self.app.device_scale_factor
            && let Ok(mut dpi) = device_scale_factor.lock()
        {
            *dpi = current_dpi;
        }

        if let Some(browser) = self.app.browser.as_mut()
            && let Some(host) = browser.host()
        {
            host.notify_screen_info_changed();
            host.was_resized();
        }

        self.app.last_size = logical_size;
        self.app.last_dpi = current_dpi;
        true
    }

    fn update_texture(&mut self) {
        if let Some(RenderMode::Software {
            frame_buffer,
            texture,
        }) = &mut self.app.render_mode
        {
            let Ok(mut fb) = frame_buffer.lock() else {
                return;
            };
            if !fb.dirty || fb.data.is_empty() {
                return;
            }

            let width = fb.width as i32;
            let height = fb.height as i32;
            let byte_array = PackedByteArray::from(fb.data.as_slice());

            let image: Option<Gd<Image>> =
                Image::create_from_data(width, height, false, ImageFormat::RGBA8, &byte_array);
            if let Some(image) = image {
                texture.set_image(&image);
            }

            fb.mark_clean();
            return;
        }

        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            let needs_resize = if let Some(RenderMode::Accelerated {
                texture_info,
                texture_width,
                texture_height,
                ..
            }) = &self.app.render_mode
            {
                if let Ok(tex_info) = texture_info.lock() {
                    tex_info.width != *texture_width || tex_info.height != *texture_height
                } else {
                    false
                }
            } else {
                false
            };

            if needs_resize {
                let old_rd_rid = if let Some(RenderMode::Accelerated { rd_texture_rid, .. }) =
                    &self.app.render_mode
                {
                    Some(*rd_texture_rid)
                } else {
                    None
                };

                let new_texture_clone = if let Some(RenderMode::Accelerated {
                    texture_info,
                    texture_width,
                    texture_height,
                    rd_texture_rid,
                    texture_2d_rd,
                    ..
                }) = &mut self.app.render_mode
                {
                    if let Ok(tex_info) = texture_info.lock() {
                        let new_w = tex_info.width;
                        let new_h = tex_info.height;
                        drop(tex_info);

                        let (new_rd_rid, new_texture_2d_rd) =
                            render::create_rd_texture(new_w as i32, new_h as i32);
                        let texture_clone = new_texture_2d_rd.clone();
                        *rd_texture_rid = new_rd_rid;
                        *texture_2d_rd = new_texture_2d_rd;
                        *texture_width = new_w;
                        *texture_height = new_h;
                        Some(texture_clone)
                    } else {
                        None
                    }
                } else {
                    None
                };

                if let Some(old_rid) = old_rd_rid {
                    render::free_rd_texture(old_rid);
                }

                if let Some(texture) = new_texture_clone {
                    self.base_mut().set_texture(&texture);
                }
            }

            if let Some(RenderMode::Accelerated {
                texture_info,
                importer,
                rd_texture_rid,
                ..
            }) = &mut self.app.render_mode
            {
                let Ok(mut tex_info) = texture_info.lock() else {
                    return;
                };

                if !tex_info.dirty
                    || !tex_info.native_handle().is_valid()
                    || tex_info.width == 0
                    || tex_info.height == 0
                {
                    tex_info.dirty = false;
                    return;
                }

                if !rd_texture_rid.is_valid() {
                    godot::global::godot_warn!("[CefTexture] RD texture RID is invalid for copy");
                    tex_info.dirty = false;
                    return;
                }

                match importer.copy_texture(&tex_info, *rd_texture_rid) {
                    Ok(()) => {}
                    Err(e) => {
                        godot::global::godot_error!("[CefTexture] GPU texture copy failed: {}", e);
                    }
                }

                tex_info.dirty = false;
            }
        }
    }

    fn request_external_begin_frame(&mut self) {
        if let Some(browser) = self.app.browser.as_mut()
            && let Some(host) = browser.host()
        {
            host.send_external_begin_frame();
        }
    }

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

    fn process_message_queue(&mut self) {
        let Some(queue) = &self.app.message_queue else {
            return;
        };

        let messages: Vec<String> = {
            let Ok(mut q) = queue.lock() else {
                return;
            };
            q.drain(..).collect()
        };

        for message in messages {
            self.base_mut()
                .emit_signal("ipc_message", &[GString::from(&message).to_variant()]);
        }
    }

    fn process_url_change_queue(&mut self) {
        let Some(queue) = &self.app.url_change_queue else {
            return;
        };

        let urls: Vec<String> = {
            let Ok(mut q) = queue.lock() else {
                return;
            };
            q.drain(..).collect()
        };

        for url in urls {
            self.base_mut()
                .emit_signal("url_changed", &[GString::from(&url).to_variant()]);
        }
    }

    fn handle_input_event(&mut self, event: Gd<InputEvent>) {
        let Some(browser) = self.app.browser.as_mut() else {
            return;
        };
        let Some(host) = browser.host() else {
            return;
        };

        if let Ok(mouse_button) = event.clone().try_cast::<InputEventMouseButton>() {
            input::handle_mouse_button(
                &host,
                &mouse_button,
                self.get_pixel_scale_factor(),
                self.get_device_scale_factor(),
            );
        } else if let Ok(mouse_motion) = event.clone().try_cast::<InputEventMouseMotion>() {
            input::handle_mouse_motion(
                &host,
                &mouse_motion,
                self.get_pixel_scale_factor(),
                self.get_device_scale_factor(),
            );
        } else if let Ok(pan_gesture) = event.clone().try_cast::<InputEventPanGesture>() {
            input::handle_pan_gesture(
                &host,
                &pan_gesture,
                self.get_pixel_scale_factor(),
                self.get_device_scale_factor(),
            );
        } else if let Ok(key_event) = event.try_cast::<InputEventKey>() {
            input::handle_key_event(&host, &key_event);
        }
    }

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

    #[func]
    pub fn eval(&mut self, code: GString) {
        let Some(browser) = self.app.browser.as_ref() else {
            godot::global::godot_warn!("[CefTexture] Cannot execute JS: no browser");
            return;
        };
        let Some(frame) = browser.main_frame() else {
            godot::global::godot_warn!("[CefTexture] Cannot execute JS: no main frame");
            return;
        };

        let code_str: cef::CefStringUtf16 = code.to_string().as_str().into();
        frame.execute_java_script(Some(&code_str), None, 0);
    }

    #[func]
    pub fn load_url(&mut self, url: GString) {
        let Some(browser) = self.app.browser.as_ref() else {
            godot::global::godot_warn!("[CefTexture] Cannot load URL: no browser");
            return;
        };
        let Some(frame) = browser.main_frame() else {
            godot::global::godot_warn!("[CefTexture] Cannot load URL: no main frame");
            return;
        };

        let url_str: cef::CefStringUtf16 = url.to_string().as_str().into();
        frame.load_url(Some(&url_str));
    }

    #[func]
    /// Sends a message into the page via `window.onIpcMessage`.
    ///
    /// This is intentionally separate from [`eval`]: callers could achieve a
    /// similar effect with `eval("window.onIpcMessage(...);")`, but this
    /// helper:
    /// - automatically escapes the string payload for safe JS embedding, and
    /// - enforces a consistent IPC pattern (`window.onIpcMessage(message)`).
    ///
    /// Use this when you want structured IPC into the page, and `eval` when
    /// you truly need arbitrary JavaScript execution.
    pub fn send_ipc_message(&mut self, message: GString) {
        let Some(browser) = self.app.browser.as_ref() else {
            godot::global::godot_warn!("[CefTexture] Cannot send IPC message: no browser");
            return;
        };
        let Some(frame) = browser.main_frame() else {
            godot::global::godot_warn!("[CefTexture] Cannot send IPC message: no main frame");
            return;
        };

        // Use serde_json for proper JSON encoding which handles all edge cases:
        // - Unicode line terminators (U+2028, U+2029) that can break JS strings
        // - Backticks, single quotes, and all control characters
        // - Proper backslash and quote escaping
        // The result includes surrounding quotes, so we use it directly.
        let msg_str = message.to_string();
        let json_msg = serde_json::to_string(&msg_str).unwrap_or_else(|_| "\"\"".to_string());

        let js_code = format!(
            r#"if (typeof window.onIpcMessage === 'function') {{ window.onIpcMessage({}); }}"#,
            json_msg
        );
        let js_code_str: cef::CefStringUtf16 = js_code.as_str().into();
        frame.execute_java_script(Some(&js_code_str), None, 0);
    }

    fn on_focus_enter(&mut self) {
        let Some(browser) = self.app.browser.as_mut() else {
            return;
        };
        let Some(host) = browser.host() else {
            return;
        };
        host.set_focus(true as _);
    }

    fn on_focus_exit(&mut self) {
        let Some(browser) = self.app.browser.as_mut() else {
            return;
        };
        let Some(host) = browser.host() else {
            return;
        };
        host.set_focus(false as _);
    }
}
