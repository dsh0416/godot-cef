use super::CefTexture;
use cef::{ImplBrowser, ImplBrowserHost};
use godot::classes::image::Format as ImageFormat;
use godot::classes::{DisplayServer, Engine, Image};
use godot::prelude::*;

use crate::browser::RenderMode;
use crate::{cursor, render};

impl CefTexture {
    pub(super) fn get_max_fps(&self) -> i32 {
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

    pub(super) fn handle_max_fps_change(&mut self) {
        let max_fps = self.get_max_fps();
        if max_fps == self.last_max_fps {
            return;
        }

        self.last_max_fps = max_fps;
        if let Some(browser) = self.app.browser.as_mut()
            && let Some(host) = browser.host()
        {
            host.set_windowless_frame_rate(max_fps);
        }
    }

    pub(super) fn handle_size_change(&mut self) -> bool {
        let current_dpi = self.get_pixel_scale_factor();
        let logical_size = self.base().get_size();
        if logical_size.x <= 0.0 || logical_size.y <= 0.0 {
            return false;
        }

        let size_diff = (logical_size - self.last_size).abs();
        let dpi_diff = (current_dpi - self.last_dpi).abs();
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

        self.last_size = logical_size;
        self.last_dpi = current_dpi;
        true
    }

    pub(super) fn update_texture(&mut self) {
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
        if let Some(RenderMode::Accelerated {
            render_state,
            texture_2d_rd,
        }) = &mut self.app.render_mode
        {
            let Ok(mut state) = render_state.lock() else {
                return;
            };

            if let Some((new_w, new_h)) = state.needs_resize.take()
                && new_w > 0
                && new_h > 0
            {
                render::free_rd_texture(state.dst_rd_rid);

                let (new_rd_rid, new_texture_2d_rd) =
                    match render::create_rd_texture(new_w as i32, new_h as i32) {
                        Ok(result) => result,
                        Err(e) => {
                            godot::global::godot_error!("[CefTexture] {}", e);
                            return;
                        }
                    };

                state.dst_rd_rid = new_rd_rid;
                state.dst_width = new_w;
                state.dst_height = new_h;

                *texture_2d_rd = new_texture_2d_rd.clone();
                drop(state);

                self.base_mut().set_texture(&new_texture_2d_rd);
            }
        }
    }

    pub(super) fn request_external_begin_frame(&mut self) {
        if let Some(browser) = self.app.browser.as_mut()
            && let Some(host) = browser.host()
        {
            host.send_external_begin_frame();
        }
    }

    pub(super) fn update_cursor(&mut self) {
        let Some(cursor_type_arc) = &self.app.cursor_type else {
            return;
        };

        let current_cursor = match cursor_type_arc.lock() {
            Ok(cursor_type) => *cursor_type,
            Err(_) => return,
        };

        if current_cursor == self.last_cursor {
            return;
        }

        self.last_cursor = current_cursor;
        let shape = cursor::cursor_type_to_shape(current_cursor);
        self.base_mut().set_default_cursor_shape(shape);
    }
}
