mod accelerated_osr;
mod cef_init;
mod cursor;
mod input;
mod utils;
mod webrender;

use cef::{
    BrowserSettings, ImplBrowser, ImplBrowserHost, RequestContextSettings, WindowInfo, api_hash,
    do_message_loop_work,
};
use cef_app::{CursorType, FrameBuffer};
use godot::classes::image::Format as ImageFormat;
use godot::classes::notify::ControlNotification;
use godot::classes::texture_rect::ExpandMode;
use godot::classes::{
    DisplayServer, Engine, ITextureRect, Image, ImageTexture, InputEvent, InputEventKey,
    InputEventMouseButton, InputEventMouseMotion, InputEventPanGesture, RenderingServer,
    Shader, ShaderMaterial, TextureRect,
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

/// A TextureRect that creates and manages a Godot-owned texture suitable for
/// GPU-to-GPU copying from external sources (like CEF shared textures).
/// 
/// This node creates an ImageTexture with a placeholder Image that has the correct
/// usage flags for Godot's rendering pipeline. The native handle of this texture
/// can be obtained via RenderingDevice::get_driver_resource() for direct GPU copying.
#[derive(GodotClass)]
#[class(base=TextureRect)]
pub struct TextureRectRd {
    base: Base<TextureRect>,
    texture: Option<Gd<ImageTexture>>,
    width: u32,
    height: u32,
}

#[godot_api]
impl ITextureRect for TextureRectRd {
    fn init(base: Base<TextureRect>) -> Self {
        Self {
            base,
            texture: None,
            width: 0,
            height: 0,
        }
    }

    fn ready(&mut self) {
        self.base_mut().set_expand_mode(ExpandMode::IGNORE_SIZE);
    }
}

#[godot_api]
impl TextureRectRd {
    /// Creates or resizes the internal texture to the specified dimensions.
    /// Returns the RID of the texture for use with RenderingServer operations.
    #[func]
    pub fn ensure_texture_size(&mut self, width: i32, height: i32) -> Rid {
        let width = width.max(1) as u32;
        let height = height.max(1) as u32;

        // Only recreate if dimensions changed
        if self.width == width && self.height == height {
            if let Some(ref texture) = self.texture {
                return texture.get_rid();
            }
        }

        self.width = width;
        self.height = height;

        // Create a placeholder image with the specified dimensions
        let image = Image::create(width as i32, height as i32, false, ImageFormat::RGBA8);
        
        if let Some(image) = image {
            let mut texture = ImageTexture::new_gd();
            texture.set_image(&image);
            
            // Get the RID before storing
            let rid = texture.get_rid();
            
            // Set this texture on the TextureRect
            self.base_mut().set_texture(&texture);
            self.texture = Some(texture);
            
            rid
        } else {
            godot::global::godot_error!("[TextureRectRd] Failed to create placeholder image {}x{}", width, height);
            Rid::Invalid
        }
    }

    /// Returns the RID of the internal texture, or Invalid if no texture exists.
    #[func]
    pub fn get_texture_rid(&self) -> Rid {
        self.texture.as_ref().map(|t| t.get_rid()).unwrap_or(Rid::Invalid)
    }

    /// Returns the RenderingDevice RID for the texture, which can be used with
    /// get_driver_resource() to obtain the native handle.
    #[func]
    pub fn get_rd_texture_rid(&self) -> Rid {
        let texture_rid = self.get_texture_rid();
        if !texture_rid.is_valid() {
            return Rid::Invalid;
        }
        
        let rs = RenderingServer::singleton();
        rs.texture_get_rd_texture(texture_rid)
    }

    /// Returns the current texture width.
    #[func]
    pub fn get_texture_width(&self) -> i32 {
        self.width as i32
    }

    /// Returns the current texture height.
    #[func]
    pub fn get_texture_height(&self) -> i32 {
        self.height as i32
    }
}

/// Shader code to swap BGRA to RGBA for CEF textures
const COLOR_SWAP_SHADER_CODE: &str = r#"
shader_type canvas_item;

void fragment() {
    vec4 tex_color = texture(TEXTURE, UV);
    COLOR = vec4(tex_color.b, tex_color.g, tex_color.r, tex_color.a);
}
"#;

enum RenderMode {
    Software {
        frame_buffer: Arc<Mutex<FrameBuffer>>,
        texture: Gd<ImageTexture>,
    },
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    Accelerated {
        texture_info: Arc<Mutex<PlatformSharedTextureInfo>>,
        importer: GodotTextureImporter,
        /// The Godot-owned texture that we copy CEF's texture into
        godot_texture: Gd<ImageTexture>,
        /// Current texture dimensions
        texture_width: u32,
        texture_height: u32,
        /// Color swap shader (BGRA -> RGBA)
        color_swap_shader: Gd<Shader>,
        /// Color swap material
        color_swap_material: Gd<ShaderMaterial>,
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
    last_max_fps: i32,
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
            last_max_fps: 0,
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
            api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
            cef_init::initialize_cef();
        });

        self.create_browser();
        // self.request_external_begin_frame();
    }

    fn on_process(&mut self) {
        self.handle_max_fps_change();
        _ = self.handle_size_change();
        self.update_texture();

        do_message_loop_work();

        self.request_external_begin_frame();
        self.update_cursor();
    }

    fn shutdown(&mut self) {
        // Note: The Godot-owned texture, shader, and material in Accelerated mode
        // will be cleaned up automatically when render_mode is set to None
        // (Gd objects are dropped)

        self.app.browser = None;
        self.app.render_mode = None;
        self.app.render_size = None;
        self.app.device_scale_factor = None;
        self.app.cursor_type = None;
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

        // Create color swap shader and material (BGRA -> RGBA)
        let mut color_swap_shader = Shader::new_gd();
        color_swap_shader.set_code(COLOR_SWAP_SHADER_CODE);
        
        let mut color_swap_material = ShaderMaterial::new_gd();
        color_swap_material.set_shader(&color_swap_shader);

        // Create a Godot-owned texture for GPU copy destination
        let godot_texture = Self::create_godot_texture(pixel_width, pixel_height);
        self.base_mut().set_texture(&godot_texture);
        
        // Apply color swap material to the TextureRect
        self.base_mut().set_material(&color_swap_material);

        self.app.render_mode = Some(RenderMode::Accelerated {
            texture_info,
            importer,
            godot_texture,
            texture_width: pixel_width as u32,
            texture_height: pixel_height as u32,
            color_swap_shader,
            color_swap_material,
        });
        self.app.render_size = Some(render_size);
        self.app.device_scale_factor = Some(device_scale_factor);
        self.app.cursor_type = Some(cursor_type);

        let mut client = webrender::AcceleratedClientImpl::build(
            render_handler,
            self.app.cursor_type.clone().unwrap(),
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

    /// Creates a Godot-owned ImageTexture with proper usage flags for GPU copying.
    fn create_godot_texture(width: i32, height: i32) -> Gd<ImageTexture> {
        let width = width.max(1);
        let height = height.max(1);

        let image = Image::create(width, height, false, ImageFormat::RGBA8)
            .expect("Failed to create placeholder image");
        
        let mut texture = ImageTexture::new_gd();
        texture.set_image(&image);
        texture
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
        DisplayServer::singleton().screen_get_scale()
    }

    fn get_max_fps(&self) -> i32 {
        let engine_cap_fps = Engine::singleton().get_max_fps();
        let screen_cap_fps = DisplayServer::singleton().screen_get_refresh_rate().round() as i32;
        if engine_cap_fps > 0 {
            engine_cap_fps
        } else {
            if screen_cap_fps > 0 {
                screen_cap_fps
            } else {
                60
            }
        }
    }

    fn handle_max_fps_change(&mut self) {
        let max_fps = self.get_max_fps();
        if max_fps == self.app.last_max_fps {
            return;
        }

        self.app.last_max_fps = max_fps;
        if let Some(browser) = self.app.browser.as_mut() {
            if let Some(host) = browser.host() {
                host.set_windowless_frame_rate(max_fps);
            }
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

        if let Some(render_size) = &self.app.render_size {
            if let Ok(mut size) = render_size.lock() {
                size.width = pixel_width;
                size.height = pixel_height;
            }
        }

        if let Some(device_scale_factor) = &self.app.device_scale_factor {
            if let Ok(mut dpi) = device_scale_factor.lock() {
                *dpi = current_dpi;
            }
        }

        if let Some(browser) = self.app.browser.as_mut() {
            if let Some(host) = browser.host() {
                host.notify_screen_info_changed();
                host.was_resized();
            }
        }

        self.app.last_size = logical_size;
        self.app.last_dpi = current_dpi;
        return true;
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
            // First, check if we need to resize and handle it separately to avoid borrow issues
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

            // Handle resize outside of the render_mode borrow
            if needs_resize {
                let new_texture_clone = if let Some(RenderMode::Accelerated {
                    texture_info,
                    texture_width,
                    texture_height,
                    godot_texture,
                    ..
                }) = &mut self.app.render_mode
                {
                    if let Ok(tex_info) = texture_info.lock() {
                        let new_w = tex_info.width;
                        let new_h = tex_info.height;
                        drop(tex_info); // Release the lock before creating texture
                        
                        // Recreate the Godot texture with new dimensions
                        let new_texture = Self::create_godot_texture(new_w as i32, new_h as i32);
                        let texture_clone = new_texture.clone();
                        *godot_texture = new_texture;
                        *texture_width = new_w;
                        *texture_height = new_h;
                        Some(texture_clone)
                    } else {
                        None
                    }
                } else {
                    None
                };
                
                // Now set the texture on base (outside of render_mode borrow)
                if let Some(texture) = new_texture_clone {
                    self.base_mut().set_texture(&texture);
                }
            }

            // Now perform the actual texture copy
            if let Some(RenderMode::Accelerated {
                texture_info,
                importer,
                godot_texture,
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

                // Get the RenderingDevice RID for the destination texture
                let texture_rid = godot_texture.get_rid();
                let rs = RenderingServer::singleton();
                let rd_texture_rid = rs.texture_get_rd_texture(texture_rid);

                if !rd_texture_rid.is_valid() {
                    godot::global::godot_warn!(
                        "[CefTexture] Failed to get RD texture RID for copy"
                    );
                    tex_info.dirty = false;
                    return;
                }

                // Perform GPU copy from CEF texture to Godot texture
                match importer.copy_texture(&tex_info, rd_texture_rid) {
                    Ok(()) => {
                        // Copy successful - texture is now updated
                    }
                    Err(e) => {
                        godot::global::godot_error!(
                            "[CefTexture] GPU texture copy failed: {}",
                            e
                        );
                    }
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
}
