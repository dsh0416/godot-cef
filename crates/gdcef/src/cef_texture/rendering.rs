use super::CefTexture;
use cef::{ImplBrowser, ImplBrowserHost};
use godot::classes::control::MouseFilter;
use godot::classes::image::Format as ImageFormat;
use godot::classes::texture_rect::ExpandMode;
use godot::classes::{DisplayServer, Engine, Image, TextureRect};
use godot::prelude::*;
use software_render::{DestBuffer, PopupBuffer, composite_popup};

use crate::browser::RenderMode;
use crate::utils::get_display_scale_factor;
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

            let popup_metadata = self.app.popup_state.as_ref().and_then(|ps| {
                ps.lock().ok().and_then(|popup| {
                    if popup.visible && !popup.buffer.is_empty() {
                        Some((
                            popup.width,
                            popup.height,
                            popup.rect.x,
                            popup.rect.y,
                            popup.dirty,
                        ))
                    } else {
                        None
                    }
                })
            });

            let popup_dirty = popup_metadata
                .as_ref()
                .is_some_and(|(_, _, _, _, dirty)| *dirty);

            if !fb.dirty && !popup_dirty {
                return;
            }

            if fb.data.is_empty() {
                return;
            }

            let width = fb.width as i32;
            let height = fb.height as i32;
            let display_scale = get_display_scale_factor();

            let final_data =
                if let Some((popup_width, popup_height, popup_x, popup_y, _)) = popup_metadata {
                    let popup_buffer = self
                        .app
                        .popup_state
                        .as_ref()
                        .and_then(|ps| ps.lock().ok().map(|popup| popup.buffer.clone()));

                    if let Some(popup_buffer) = popup_buffer {
                        let mut composited = fb.data.clone();
                        let scaled_x = (popup_x as f32 * display_scale) as i32;
                        let scaled_y = (popup_y as f32 * display_scale) as i32;
                        composite_popup(
                            &mut DestBuffer {
                                data: &mut composited,
                                width: fb.width,
                                height: fb.height,
                            },
                            &PopupBuffer {
                                data: &popup_buffer,
                                width: popup_width,
                                height: popup_height,
                                x: scaled_x,
                                y: scaled_y,
                            },
                        );
                        if let Some(ps) = &self.app.popup_state
                            && let Ok(mut popup) = ps.lock()
                        {
                            popup.mark_clean();
                        }
                        composited
                    } else {
                        fb.data.clone()
                    }
                } else {
                    fb.data.clone()
                };

            let byte_array = PackedByteArray::from(final_data.as_slice());

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

            let texture_to_set = if let Some((new_w, new_h)) = state.needs_resize.take()
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
                Some(new_texture_2d_rd)
            } else {
                None
            };

            if state.has_pending_copy
                && let Err(e) = state.process_pending_copy()
            {
                godot::global::godot_error!("[CefTexture] Failed to process pending copy: {}", e);
            }

            drop(state);

            if let Some(tex) = texture_to_set {
                self.base_mut().set_texture(&tex);
            }
        }

        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        if let Some(RenderMode::Accelerated { render_state, .. }) = &self.app.render_mode {
            if let Ok(mut state) = render_state.lock()
                && let Some((new_w, new_h)) = state.needs_popup_texture.take()
            {
                if let Some(old_rid) = state.popup_rd_rid {
                    render::free_rd_texture(old_rid);
                }

                match render::create_rd_texture(new_w as i32, new_h as i32) {
                    Ok((new_rid, new_texture_2d_rd)) => {
                        state.popup_rd_rid = Some(new_rid);
                        state.popup_width = new_w;
                        state.popup_height = new_h;
                        self.popup_texture_2d_rd = Some(new_texture_2d_rd);
                    }
                    Err(e) => {
                        godot::global::godot_error!(
                            "[CefTexture] Failed to create popup texture: {}",
                            e
                        );
                    }
                }
            }

            self.update_popup_overlay();
        }
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn update_popup_overlay(&mut self) {
        let popup_visible_info = self.app.popup_state.as_ref().and_then(|ps| {
            ps.lock().ok().and_then(|popup| {
                if popup.visible {
                    Some((
                        popup.rect.x,
                        popup.rect.y,
                        popup.rect.width,
                        popup.rect.height,
                    ))
                } else {
                    None
                }
            })
        });

        let accel_popup_info = if let Some(RenderMode::Accelerated { render_state, .. }) =
            &self.app.render_mode
        {
            render_state.lock().ok().and_then(|state| {
                if state.popup_rd_rid.is_some() && state.popup_width > 0 && state.popup_height > 0 {
                    Some((
                        state.popup_dirty,
                        state.popup_has_content,
                        state.popup_width,
                        state.popup_height,
                    ))
                } else {
                    None
                }
            })
        } else {
            None
        };

        match (popup_visible_info, accel_popup_info) {
            (
                Some((x, y, _rect_w, _rect_h)),
                Some((popup_dirty, popup_has_content, tex_width, tex_height)),
            ) => {
                if self.popup_overlay.is_none() {
                    let mut overlay = TextureRect::new_alloc();
                    overlay.set_expand_mode(ExpandMode::IGNORE_SIZE);
                    overlay.set_mouse_filter(MouseFilter::IGNORE);
                    let overlay_node: Gd<godot::classes::Node> = overlay.clone().upcast();
                    self.base_mut().add_child(&overlay_node);
                    self.popup_overlay = Some(overlay);
                }

                let display_scale = get_display_scale_factor();
                let cef_texture_size = self.base().get_size();
                let render_size = self
                    .app
                    .render_size
                    .as_ref()
                    .and_then(|s| s.lock().ok().map(|sz| (sz.width, sz.height)))
                    .unwrap_or((0.0, 0.0));

                if let Some(overlay) = &mut self.popup_overlay {
                    if let Some(texture_2d_rd) = &self.popup_texture_2d_rd {
                        overlay.set_texture(texture_2d_rd);
                    }

                    let scale_x = if render_size.0 > 0.0 {
                        cef_texture_size.x * display_scale / render_size.0
                    } else {
                        display_scale
                    };
                    let scale_y = if render_size.1 > 0.0 {
                        cef_texture_size.y * display_scale / render_size.1
                    } else {
                        display_scale
                    };

                    let local_x = x as f32 * scale_x;
                    let local_y = y as f32 * scale_y;
                    let local_width = tex_width as f32 * scale_x / display_scale;
                    let local_height = tex_height as f32 * scale_y / display_scale;

                    overlay.set_position(Vector2::new(local_x, local_y));
                    overlay.set_size(Vector2::new(local_width, local_height));
                    overlay.set_visible(popup_has_content);
                }

                if popup_dirty
                    && let Some(RenderMode::Accelerated { render_state, .. }) =
                        &self.app.render_mode
                    && let Ok(mut state) = render_state.lock()
                {
                    state.popup_dirty = false;
                }
            }
            _ => {
                if let Some(overlay) = &mut self.popup_overlay {
                    overlay.set_visible(false);
                }
                if let Some(RenderMode::Accelerated { render_state, .. }) = &self.app.render_mode
                    && let Ok(mut state) = render_state.lock()
                {
                    state.popup_has_content = false;
                }
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
