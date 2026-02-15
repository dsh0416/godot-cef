use cef::{ImplBrowser, ImplFrame};
use godot::classes::image::Format as ImageFormat;
use godot::classes::{DisplayServer, ITexture2D, Image, RenderingServer, Texture2D};
use godot::prelude::*;
use software_render::{DestBuffer, composite_popup};

use crate::browser::{App, RenderMode};
use crate::cef_texture::backend;
use crate::{cef_init, render};

#[derive(GodotClass)]
#[class(base=Texture2D)]
pub struct CefTexture2D {
    base: Base<Texture2D>,
    app: App,

    #[export]
    #[var(get = get_url_property, set = set_url_property)]
    url: GString,

    #[export]
    enable_accelerated_osr: bool,

    #[export]
    background_color: Color,

    #[export(enum = (Block = 0, Redirect = 1, SignalOnly = 2))]
    #[var(get = get_popup_policy, set = set_popup_policy)]
    popup_policy: i32,

    #[export]
    #[var(get = get_texture_size_property, set = set_texture_size_property)]
    texture_size: Vector2i,

    last_size: Vector2,
    last_dpi: f32,
    last_max_fps: i32,
    frame_hook_connected: bool,
}

#[godot_api]
impl ITexture2D for CefTexture2D {
    fn init(base: Base<Texture2D>) -> Self {
        Self {
            base,
            app: App::default(),
            url: "https://google.com".into(),
            enable_accelerated_osr: true,
            background_color: Color::from_rgba(0.0, 0.0, 0.0, 0.0),
            popup_policy: crate::browser::popup_policy::BLOCK,
            texture_size: Vector2i::new(1024, 1024),
            last_size: Vector2::ZERO,
            last_dpi: 1.0,
            last_max_fps: 0,
            frame_hook_connected: false,
        }
    }

    fn get_width(&self) -> i32 {
        self.texture_size.x
    }

    fn get_height(&self) -> i32 {
        self.texture_size.y
    }

    fn has_alpha(&self) -> bool {
        true
    }
}

#[godot_api]
impl CefTexture2D {
    #[func]
    fn _on_frame_pre_draw(&mut self) {
        self.tick();
    }

    #[func]
    fn set_url_property(&mut self, url: GString) {
        self.url = url.clone();
        if let Some(state) = self.app.state.as_ref()
            && let Some(frame) = state.browser.main_frame()
        {
            let url_str: cef::CefStringUtf16 = url.to_string().as_str().into();
            frame.load_url(Some(&url_str));
        }
    }

    #[func]
    fn get_url_property(&self) -> GString {
        if let Some(state) = self.app.state.as_ref()
            && let Some(frame) = state.browser.main_frame()
        {
            let frame_url = frame.url();
            let url_string = cef::CefStringUtf16::from(&frame_url).to_string();
            return GString::from(url_string.as_str());
        }
        self.url.clone()
    }

    #[func]
    fn get_popup_policy(&self) -> i32 {
        self.popup_policy
    }

    #[func]
    fn set_popup_policy(&mut self, policy: i32) {
        self.popup_policy = policy;
        backend::apply_popup_policy(&self.app, policy);
    }

    #[func]
    fn get_texture_size_property(&self) -> Vector2i {
        self.texture_size
    }

    #[func]
    fn set_texture_size_property(&mut self, size: Vector2i) {
        self.texture_size = Vector2i::new(size.x.max(1), size.y.max(1));
    }

    #[func]
    pub fn shutdown(&mut self) {
        self.cleanup_instance();
    }

    fn ensure_frame_hook(&mut self) {
        if self.frame_hook_connected {
            return;
        }
        let callable = self.base().callable("_on_frame_pre_draw");
        RenderingServer::singleton().connect("frame_pre_draw", &callable);
        self.frame_hook_connected = true;
    }

    fn get_max_fps(&self) -> i32 {
        let setting_fps = crate::settings::get_max_frame_rate();
        if setting_fps > 0 {
            return setting_fps;
        }
        let engine_cap_fps = godot::classes::Engine::singleton().get_max_fps();
        let screen_cap_fps = DisplayServer::singleton().screen_get_refresh_rate().round() as i32;
        if engine_cap_fps > 0 {
            engine_cap_fps
        } else if screen_cap_fps > 0 {
            screen_cap_fps
        } else {
            60
        }
    }

    fn get_dpi(&self) -> f32 {
        crate::utils::get_display_scale_factor()
    }

    fn logical_size(&self) -> Vector2 {
        Vector2::new(self.texture_size.x as f32, self.texture_size.y as f32)
    }

    fn try_create_browser(&mut self) {
        if self.app.state.is_some() {
            return;
        }
        if let Err(e) = cef_init::cef_retain() {
            godot::global::godot_error!("[CefTexture2D] {}", e);
            return;
        }
        let logical_size = self.logical_size();
        let dpi = self.get_dpi();
        let params = backend::BackendCreateParams {
            logical_size,
            dpi,
            max_fps: self.get_max_fps(),
            url: self.url.to_string(),
            enable_accelerated_osr: self.enable_accelerated_osr,
            background_color: self.background_color,
            popup_policy: self.popup_policy,
            log_prefix: "CefTexture2D",
        };
        if let Err(e) = backend::try_create_browser(&mut self.app, &params) {
            godot::global::godot_error!("[CefTexture2D] {}", e);
            cef_init::cef_release();
            return;
        }
        self.last_size = logical_size;
        self.last_dpi = dpi;
    }

    fn cleanup_instance(&mut self) {
        backend::cleanup_runtime(&mut self.app, None);
    }

    fn update_texture(&mut self) {
        let Some(state) = &mut self.app.state else {
            return;
        };

        if let RenderMode::Software {
            frame_buffer,
            texture,
        } = &mut state.render_mode
        {
            let Ok(mut fb) = frame_buffer.lock() else {
                return;
            };
            let popup_metadata = state.popup_state.lock().ok().and_then(|popup| {
                if popup.visible && !popup.buffer.is_empty() {
                    Some((popup.width, popup.height, popup.rect.x, popup.rect.y))
                } else {
                    None
                }
            });

            if !fb.dirty && popup_metadata.is_none() {
                return;
            }
            if fb.data.is_empty() {
                return;
            }

            let mut final_data = fb.data.clone();
            if let Some((popup_width, popup_height, popup_x, popup_y)) = popup_metadata {
                let popup_buffer = state
                    .popup_state
                    .lock()
                    .ok()
                    .map(|popup| popup.buffer.clone());
                if let Some(popup_buffer) = popup_buffer {
                    composite_popup(
                        &mut DestBuffer {
                            data: &mut final_data,
                            width: fb.width,
                            height: fb.height,
                        },
                        &software_render::PopupBuffer {
                            data: &popup_buffer,
                            width: popup_width,
                            height: popup_height,
                            x: popup_x,
                            y: popup_y,
                        },
                    );
                }
            }

            let byte_array = PackedByteArray::from(final_data.as_slice());
            let image = Image::create_from_data(
                fb.width as i32,
                fb.height as i32,
                false,
                ImageFormat::RGBA8,
                &byte_array,
            );
            if let Some(image) = image {
                texture.set_image(&image);
            }
            fb.mark_clean();
            return;
        }

        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        if let RenderMode::Accelerated {
            render_state,
            texture_2d_rd,
        } = &mut state.render_mode
        {
            let Ok(mut accel_state) = render_state.lock() else {
                return;
            };
            if let Some((new_w, new_h)) = accel_state.needs_resize.take()
                && new_w > 0
                && new_h > 0
            {
                render::free_rd_texture(accel_state.dst_rd_rid);
                let (new_rd_rid, new_texture_2d_rd) =
                    match render::create_rd_texture(new_w as i32, new_h as i32) {
                        Ok(result) => result,
                        Err(e) => {
                            godot::global::godot_error!("[CefTexture2D] {}", e);
                            return;
                        }
                    };
                accel_state.dst_rd_rid = new_rd_rid;
                accel_state.dst_width = new_w;
                accel_state.dst_height = new_h;
                *texture_2d_rd = new_texture_2d_rd;
            }

            if accel_state.has_pending_copy
                && let Err(e) = accel_state.process_pending_copy()
            {
                godot::global::godot_error!("[CefTexture2D] Failed to process pending copy: {}", e);
            }
        }
    }

    fn tick(&mut self) {
        self.ensure_frame_hook();
        self.try_create_browser();

        let max_fps = self.get_max_fps();
        backend::handle_max_fps_change(&self.app, &mut self.last_max_fps, max_fps);
        let logical_size = self.logical_size();
        let dpi = self.get_dpi();
        let _ = backend::handle_size_change(
            &self.app,
            &mut self.last_size,
            &mut self.last_dpi,
            logical_size,
            dpi,
        );
        self.update_texture();
        cef::do_message_loop_work();
        backend::request_external_begin_frame(&self.app);
    }
}

impl Drop for CefTexture2D {
    fn drop(&mut self) {
        self.cleanup_instance();
    }
}
