mod accelerated_osr;
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
    InputEventMouseMotion, InputEventPanGesture, RenderingServer, TextureRect,
};
use godot::init::*;
use godot::prelude::*;
use std::sync::{Arc, Mutex};
use winit::dpi::PhysicalSize;

use crate::accelerated_osr::{
    GodotTextureImporter, NativeHandleTrait, PlatformAcceleratedRenderHandler,
    PlatformSharedTextureInfo, TextureImporterTrait,
};
use crate::cef_init::CEF_INITIALIZED;

struct GodotCef;

#[gdextension]
unsafe impl ExtensionLibrary for GodotCef {}

enum RenderMode {
    Software {
        frame_buffer: Arc<Mutex<FrameBuffer>>,
        texture: Gd<ImageTexture>,
    },
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    Accelerated {
        texture_info: Arc<Mutex<PlatformSharedTextureInfo>>,
        importer: GodotTextureImporter,
        current_rid: Option<Rid>,
    },
}

struct App {
    browser: Option<cef::Browser>,
    render_mode: Option<RenderMode>,
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
            render_mode: None,
            render_size: None,
            device_scale_factor: None,
            cursor_type: None,
            last_size: Vector2::ZERO,
            last_dpi: 1.0,
            last_cursor: CursorType::Arrow,
        }
    }
}

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

        self.update_texture();
        self.update_cursor();
        self.request_external_begin_frame();
    }

    fn shutdown(&mut self) {
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        if let Some(RenderMode::Accelerated { current_rid, .. }) = &self.app.render_mode {
            if let Some(rid) = current_rid {
                let mut rs = RenderingServer::singleton();
                rs.free_rid(*rid);
            }
        }

        self.app.browser = None;
        self.app.render_mode = None;
        self.app.render_size = None;
        self.app.device_scale_factor = None;
        self.app.cursor_type = None;

        cef_init::shutdown_cef();
    }

    fn create_browser(&mut self) {
        let logical_size = self.base().get_rect().size;
        let dpi = self.get_content_scale_factor();
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

        let browser_settings = BrowserSettings::default();

        let mut context = cef::request_context_create_context(
            Some(&RequestContextSettings::default()),
            Some(&mut webrender::RequestContextHandlerImpl::build(
                webrender::OsrRequestContextHandler {},
            )),
        );

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

        let texture = ImageTexture::new_gd();
        self.base_mut().set_texture(&texture);

        self.app.render_mode = Some(RenderMode::Software {
            frame_buffer,
            texture,
        });
        self.app.render_size = Some(render_size);
        self.app.device_scale_factor = Some(device_scale_factor);
        self.app.cursor_type = Some(cursor_type);

        let mut client = webrender::SoftwareClientImpl::build(render_handler);

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

        self.app.render_mode = Some(RenderMode::Accelerated {
            texture_info,
            importer,
            current_rid: None,
        });
        self.app.render_size = Some(render_size);
        self.app.device_scale_factor = Some(device_scale_factor);
        self.app.cursor_type = Some(cursor_type);

        let mut client =
            webrender::AcceleratedClientImpl::build(render_handler, self.app.cursor_type.clone().unwrap());

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

            let image =
                Image::create_from_data(width, height, false, ImageFormat::RGBA8, &byte_array);
            if let Some(image) = image {
                texture.set_image(&image);
            }

            fb.mark_clean();
            return;
        }

        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            let canvas_item = self.base().get_canvas_item();
            let size = self.base().get_size();

            if let Some(RenderMode::Accelerated {
                texture_info,
                importer,
                current_rid,
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

                let color_swap_material = importer.get_color_swap_material();

                if let Some(new_rid) = importer.import_texture(&tex_info) {
                    *current_rid = Some(new_rid);

                    let mut rs = RenderingServer::singleton();
                    rs.canvas_item_clear(canvas_item);

                    if let Some(material_rid) = color_swap_material {
                        rs.canvas_item_set_material(canvas_item, material_rid);
                    }

                    rs.canvas_item_add_texture_rect(
                        canvas_item,
                        Rect2::new(Vector2::ZERO, size),
                        new_rid,
                    );
                }

                tex_info.dirty = false;
            }
        }
    }

    fn request_external_begin_frame(&mut self) {
        if let Some(browser) = self.app.browser.as_mut() {
            if let Some(host) = browser.host() {
                host.send_external_begin_frame();
            }
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
        } else if let Ok(pan_gesture) = event.clone().try_cast::<InputEventPanGesture>() {
            input::handle_pan_gesture(&host, &pan_gesture, dpi);
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
}
