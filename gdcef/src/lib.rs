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
    do_message_loop_work,
};
use godot::classes::image::Format as ImageFormat;
use godot::classes::notify::ControlNotification;
use godot::classes::texture_rect::ExpandMode;
use godot::classes::{
    DisplayServer, Engine, ITextureRect, Image, ImageTexture, InputEvent, InputEventKey,
    InputEventMouseButton, InputEventMouseMotion, InputEventPanGesture, LineEdit, TextureRect,
};
use godot::init::*;
use godot::prelude::*;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use winit::dpi::PhysicalSize;

use crate::accelerated_osr::{
    GodotTextureImporter, NativeHandleTrait, PlatformAcceleratedRenderHandler, TextureImporterTrait,
};
use crate::browser::{
    App, ImeCompositionQueue, ImeEnableQueue, LoadingStateEvent, LoadingStateQueue, MessageQueue,
    RenderMode, TitleChangeQueue, UrlChangeQueue,
};
use crate::utils::get_display_scale_factor;

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
    #[var(get = get_url_property, set = set_url_property)]
    url: GString,

    #[export]
    enable_accelerated_osr: bool,

    #[var]
    /// Stores the IME cursor position in local coordinates (relative to this `CefTexture` node),
    /// automatically updated from the browser's caret position.
    ime_position: Vector2i,
}

#[godot_api]
impl ITextureRect for CefTexture {
    fn init(base: Base<TextureRect>) -> Self {
        Self {
            base,
            app: App::default(),
            url: "https://google.com".into(),
            enable_accelerated_osr: true,
            ime_position: Vector2i::new(0, 0),
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
            ControlNotification::PREDELETE => {
                self.cleanup_instance();
            }
            ControlNotification::FOCUS_ENTER => {
                self.on_focus_enter();
            }
            ControlNotification::OS_IME_UPDATE => {
                self.handle_os_ime_update();
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

    #[signal]
    fn title_changed(title: GString);

    #[signal]
    fn load_started(url: GString);

    #[signal]
    fn load_finished(url: GString, http_status_code: i32);

    #[signal]
    fn load_error(url: GString, error_code: i32, error_text: GString);

    #[func]
    fn on_ready(&mut self) {
        use godot::classes::control::FocusMode;
        self.base_mut().set_expand_mode(ExpandMode::IGNORE_SIZE);
        // Must explicitly enable processing when using on_notification instead of fn process()
        self.base_mut().set_process(true);
        // Enable focus so we receive FOCUS_ENTER/EXIT notifications and can forward to CEF
        self.base_mut().set_focus_mode(FocusMode::CLICK);

        cef_init::cef_retain();

        // Create hidden LineEdit for IME proxy
        self.create_ime_proxy();
        self.create_browser();
    }

    /// Creates a hidden LineEdit to act as an IME input proxy.
    fn create_ime_proxy(&mut self) {
        use godot::classes::control::{FocusMode, MouseFilter};

        let mut line_edit = LineEdit::new_alloc();
        line_edit.set_position(Vector2::new(-10000.0, -10000.0));
        line_edit.set_size(Vector2::new(200.0, 30.0));
        line_edit.set_mouse_filter(MouseFilter::IGNORE);
        line_edit.set_focus_mode(FocusMode::ALL);
        let callable_changed = self.base().callable("on_ime_proxy_text_changed");
        line_edit.connect("text_changed", &callable_changed);

        let callable_focus_exited = self.base().callable("on_ime_proxy_focus_exited");
        line_edit.connect("focus_exited", &callable_focus_exited);

        self.base_mut().add_child(&line_edit);
        self.app.ime_proxy = Some(line_edit);
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
        self.process_title_change_queue();
        self.process_loading_state_queue();
        self.process_ime_enable_queue();
        self.process_ime_composition_queue();
        self.process_ime_position();
    }

    fn cleanup_instance(&mut self) {
        if self.app.browser.is_none() {
            cef_init::cef_release();
            return;
        }

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

        if let Some(browser) = self.app.browser.take()
            && let Some(host) = browser.host()
        {
            host.close_browser(true as _);
        }

        self.app.render_mode = None;
        self.app.render_size = None;
        self.app.device_scale_factor = None;
        self.app.cursor_type = None;
        self.app.message_queue = None;
        self.app.url_change_queue = None;
        self.app.title_change_queue = None;
        self.app.loading_state_queue = None;
        self.app.ime_enable_queue = None;
        self.app.ime_composition_range = None;

        self.app.ime_active = false;
        self.app.ime_proxy = None;

        cef_init::cef_release();
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
        let title_change_queue: TitleChangeQueue = Arc::new(Mutex::new(VecDeque::new()));
        let loading_state_queue: LoadingStateQueue = Arc::new(Mutex::new(VecDeque::new()));
        let ime_enable_queue: ImeEnableQueue = Arc::new(Mutex::new(VecDeque::new()));
        let ime_composition_queue: ImeCompositionQueue = Arc::new(Mutex::new(None));

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
        self.app.title_change_queue = Some(title_change_queue.clone());
        self.app.loading_state_queue = Some(loading_state_queue.clone());
        self.app.ime_enable_queue = Some(ime_enable_queue.clone());
        self.app.ime_composition_range = Some(ime_composition_queue.clone());

        let mut client = webrender::SoftwareClientImpl::build(
            render_handler,
            webrender::ClientQueues {
                message_queue,
                url_change_queue,
                title_change_queue,
                loading_state_queue,
                ime_enable_queue,
                ime_composition_queue,
            },
        );

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
        let title_change_queue: TitleChangeQueue = Arc::new(Mutex::new(VecDeque::new()));
        let loading_state_queue: LoadingStateQueue = Arc::new(Mutex::new(VecDeque::new()));
        let ime_enable_queue: ImeEnableQueue = Arc::new(Mutex::new(VecDeque::new()));
        let ime_composition_queue: ImeCompositionQueue = Arc::new(Mutex::new(None));

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
        self.app.title_change_queue = Some(title_change_queue.clone());
        self.app.loading_state_queue = Some(loading_state_queue.clone());
        self.app.ime_enable_queue = Some(ime_enable_queue.clone());
        self.app.ime_composition_range = Some(ime_composition_queue.clone());

        let mut client = webrender::AcceleratedClientImpl::build(
            render_handler,
            self.app.cursor_type.clone().unwrap(),
            webrender::ClientQueues {
                message_queue,
                url_change_queue,
                title_change_queue,
                loading_state_queue,
                ime_enable_queue,
                ime_composition_queue,
            },
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

    fn process_title_change_queue(&mut self) {
        let Some(queue) = &self.app.title_change_queue else {
            return;
        };

        let titles: Vec<String> = {
            let Ok(mut q) = queue.lock() else {
                return;
            };
            q.drain(..).collect()
        };

        for title in titles {
            self.base_mut()
                .emit_signal("title_changed", &[GString::from(&title).to_variant()]);
        }
    }

    fn process_loading_state_queue(&mut self) {
        let Some(queue) = &self.app.loading_state_queue else {
            return;
        };

        let events: Vec<LoadingStateEvent> = {
            let Ok(mut q) = queue.lock() else {
                return;
            };
            q.drain(..).collect()
        };

        for event in events {
            match event {
                LoadingStateEvent::Started { url } => {
                    self.base_mut()
                        .emit_signal("load_started", &[GString::from(&url).to_variant()]);
                }
                LoadingStateEvent::Finished {
                    url,
                    http_status_code,
                } => {
                    self.base_mut().emit_signal(
                        "load_finished",
                        &[
                            GString::from(&url).to_variant(),
                            http_status_code.to_variant(),
                        ],
                    );
                }
                LoadingStateEvent::Error {
                    url,
                    error_code,
                    error_text,
                } => {
                    self.base_mut().emit_signal(
                        "load_error",
                        &[
                            GString::from(&url).to_variant(),
                            error_code.to_variant(),
                            GString::from(&error_text).to_variant(),
                        ],
                    );
                }
            }
        }
    }

    fn process_ime_enable_queue(&mut self) {
        let Some(queue) = &self.app.ime_enable_queue else {
            return;
        };

        let final_req: Option<bool> = {
            let Ok(mut q) = queue.lock() else {
                return;
            };
            q.drain(..).next_back()
        };

        if let Some(enable) = final_req {
            if enable && !self.app.ime_active {
                self.activate_ime();
            } else if !enable && self.app.ime_active {
                self.deactivate_ime();
            }
        }
    }

    fn process_ime_composition_queue(&mut self) {
        let Some(queue) = &self.app.ime_composition_range else {
            return;
        };

        let range = {
            let Ok(mut q) = queue.lock() else {
                return;
            };
            q.take()
        };

        if let Some(range) = range
            && self.app.ime_active
        {
            self.set_ime_position(Vector2i::new(
                range.caret_x,
                range.caret_y + range.caret_height,
            ));
            self.process_ime_position();
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
            input::handle_key_event(&host, &key_event, self.app.ime_active);
        }
    }

    fn process_ime_position(&mut self) {
        if self.app.ime_active {
            let mut ds: Gd<DisplayServer> = DisplayServer::singleton();
            let display_scale = get_display_scale_factor();
            let pixel_scale = self.get_pixel_scale_factor();

            let rect = self.base().get_viewport_rect();
            let viewport_scaled =
                Vector2::new(rect.size.x * pixel_scale, rect.size.y * pixel_scale);
            let Some(window) = self.base().get_window() else {
                return;
            };
            let window_size = window.get_size();
            let viewport_offset = Vector2::new(
                (window_size.x as f32 - viewport_scaled.x) / 2.0 / pixel_scale,
                (window_size.y as f32 - viewport_scaled.y) / 2.0 / pixel_scale,
            );

            let node_offset = Vector2::new(
                self.base().get_global_position().x,
                self.base().get_global_position().y,
            );

            let final_ime_position = Vector2i::new(
                (self.ime_position.x as f32 * display_scale
                    + (viewport_offset.x + node_offset.x) * pixel_scale) as i32,
                (self.ime_position.y as f32 * display_scale
                    + (viewport_offset.y + node_offset.y) * pixel_scale) as i32,
            );

            ds.window_set_ime_position(final_ime_position);
        }
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
    fn set_url_property(&mut self, url: GString) {
        self.url = url.clone();

        if let Some(browser) = self.app.browser.as_ref()
            && let Some(frame) = browser.main_frame()
        {
            let url_str: cef::CefStringUtf16 = url.to_string().as_str().into();
            frame.load_url(Some(&url_str));
        }
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

    #[func]
    pub fn go_back(&mut self) {
        if let Some(browser) = self.app.browser.as_mut() {
            browser.go_back();
        }
    }

    #[func]
    pub fn go_forward(&mut self) {
        if let Some(browser) = self.app.browser.as_mut() {
            browser.go_forward();
        }
    }

    #[func]
    pub fn can_go_back(&self) -> bool {
        self.app
            .browser
            .as_ref()
            .map(|b| b.can_go_back() != 0)
            .unwrap_or(false)
    }

    #[func]
    pub fn can_go_forward(&self) -> bool {
        self.app
            .browser
            .as_ref()
            .map(|b| b.can_go_forward() != 0)
            .unwrap_or(false)
    }

    #[func]
    pub fn reload(&mut self) {
        if let Some(browser) = self.app.browser.as_mut() {
            browser.reload();
        }
    }

    #[func]
    pub fn reload_ignore_cache(&mut self) {
        if let Some(browser) = self.app.browser.as_mut() {
            browser.reload_ignore_cache();
        }
    }

    #[func]
    pub fn stop_loading(&mut self) {
        if let Some(browser) = self.app.browser.as_mut() {
            browser.stop_load();
        }
    }

    #[func]
    pub fn is_loading(&self) -> bool {
        self.app
            .browser
            .as_ref()
            .map(|b| b.is_loading() != 0)
            .unwrap_or(false)
    }

    #[func]
    fn get_url_property(&self) -> GString {
        if let Some(browser) = self.app.browser.as_ref()
            && let Some(frame) = browser.main_frame()
        {
            let frame_url = frame.url();
            let url_string = cef::CefStringUtf16::from(&frame_url).to_string();
            return GString::from(url_string.as_str());
        }
        self.url.clone()
    }

    #[func]
    pub fn set_zoom_level(&mut self, level: f64) {
        if let Some(browser) = self.app.browser.as_mut()
            && let Some(host) = browser.host()
        {
            host.set_zoom_level(level);
        }
    }

    #[func]
    pub fn get_zoom_level(&self) -> f64 {
        self.app
            .browser
            .as_ref()
            .and_then(|b| b.host())
            .map(|h| h.zoom_level())
            .unwrap_or(0.0)
    }

    #[func]
    pub fn set_audio_muted(&mut self, muted: bool) {
        if let Some(browser) = self.app.browser.as_mut()
            && let Some(host) = browser.host()
        {
            host.set_audio_muted(muted as i32);
        }
    }

    #[func]
    pub fn is_audio_muted(&self) -> bool {
        self.app
            .browser
            .as_ref()
            .and_then(|b| b.host())
            .map(|h| h.is_audio_muted() != 0)
            .unwrap_or(false)
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

    /// Called when the IME proxy LineEdit text changes during composition.
    #[func]
    fn on_ime_proxy_text_changed(&mut self, new_text: GString) {
        let Some(browser) = self.app.browser.as_mut() else {
            return;
        };

        let Some(host) = browser.host() else {
            return;
        };

        input::ime_commit_text(&host, &new_text.to_string());

        if let Some(proxy) = self.app.ime_proxy.as_mut() {
            proxy.set_text("");
        }
    }

    #[func]
    fn on_ime_proxy_focus_exited(&mut self) {
        self.deactivate_ime();
    }

    /// Activates IME by focusing the hidden LineEdit proxy.
    fn activate_ime(&mut self) {
        if self.app.ime_active {
            return;
        }

        self.base_mut().release_focus();

        if let Some(proxy) = self.app.ime_proxy.as_mut() {
            proxy.set_text("");
            proxy.grab_focus();
        }

        if let Some(browser) = self.app.browser.as_mut()
            && let Some(host) = browser.host()
        {
            host.set_focus(true as _);
        }

        self.app.ime_active = true;
    }

    /// Deactivates IME and commits any pending text.
    fn deactivate_ime(&mut self) {
        if !self.app.ime_active {
            return;
        }

        // Clear the proxy
        if let Some(proxy) = self.app.ime_proxy.as_mut() {
            proxy.set_text("");
        }

        self.app.ime_active = false;

        // Return focus to CefTexture
        self.base_mut().grab_focus();
    }

    fn handle_os_ime_update(&mut self) {
        if !self.app.ime_active {
            return;
        }

        let ime_text = DisplayServer::singleton().ime_get_text().to_string();
        let ime_selection = DisplayServer::singleton().ime_get_selection();
        let start = ime_selection.x.max(0) as u32;
        let end = ime_selection.y.max(0) as u32;

        // Update the IME composition text
        if let Some(browser) = self.app.browser.as_mut()
            && let Some(host) = browser.host()
        {
            input::ime_set_composition(&host, &ime_text, start, end);
        }
    }
}
