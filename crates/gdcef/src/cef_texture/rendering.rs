use super::{CefTexture, backend};
use godot::classes::TextureRect;
use godot::classes::control::MouseFilter;
use godot::classes::texture_rect::ExpandMode;
use godot::prelude::*;

use crate::browser::RenderMode;
use crate::utils::get_display_scale_factor;
use crate::{cursor, render};

impl CefTexture {
    pub(super) fn get_max_fps(&self) -> i32 {
        backend::get_max_fps()
    }

    pub(super) fn handle_max_fps_change(&mut self) {
        let max_fps = self.get_max_fps();
        backend::handle_max_fps_change(&self.app, &mut self.last_max_fps, max_fps);
    }

    pub(super) fn handle_size_change(&mut self) -> bool {
        let logical_size = self.base().get_size();
        let dpi = self.get_pixel_scale_factor();
        backend::handle_size_change(
            &self.app,
            &mut self.last_size,
            &mut self.last_dpi,
            logical_size,
            dpi,
        )
    }

    pub(super) fn update_texture(&mut self) {
        let Some(state) = &mut self.app.state else {
            return;
        };

        if let Some(tex) = backend::update_primary_texture(state, "CefTexture") {
            self.base_mut().set_texture(&tex);
        }

        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        if let Some(state) = &self.app.state {
            if let RenderMode::Accelerated { render_state, .. } = &state.render_mode
                && let Ok(mut accel_state) = render_state.lock()
                && let Some((new_w, new_h)) = accel_state.needs_popup_texture.take()
            {
                if let Some(old_rid) = accel_state.popup_rd_rid {
                    render::free_rd_texture(old_rid);
                }

                match render::create_rd_texture(new_w as i32, new_h as i32) {
                    Ok((new_rid, new_texture_2d_rd)) => {
                        accel_state.popup_rd_rid = Some(new_rid);
                        accel_state.popup_width = new_w;
                        accel_state.popup_height = new_h;
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
        let popup_visible_info = self.app.state.as_ref().and_then(|s| {
            s.popup_state.lock().ok().and_then(|popup| {
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

        let accel_popup_info = self.app.state.as_ref().and_then(|s| {
            if let RenderMode::Accelerated { render_state, .. } = &s.render_mode {
                render_state.lock().ok().and_then(|state| {
                    if state.popup_rd_rid.is_some()
                        && state.popup_width > 0
                        && state.popup_height > 0
                    {
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
            }
        });

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
                    .state
                    .as_ref()
                    .and_then(|s| s.render_size.lock().ok().map(|sz| (sz.width, sz.height)))
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
                    && let Some(s) = self.app.state.as_ref()
                    && let RenderMode::Accelerated { render_state, .. } = &s.render_mode
                    && let Ok(mut rs) = render_state.lock()
                {
                    rs.popup_dirty = false;
                }
            }
            _ => {
                if let Some(overlay) = &mut self.popup_overlay {
                    overlay.set_visible(false);
                }
                if let Some(s) = self.app.state.as_ref()
                    && let RenderMode::Accelerated { render_state, .. } = &s.render_mode
                    && let Ok(mut rs) = render_state.lock()
                {
                    rs.popup_has_content = false;
                }
            }
        }
    }

    pub(super) fn request_external_begin_frame(&mut self) {
        backend::request_external_begin_frame(&self.app);
    }

    pub(super) fn update_cursor(&mut self) {
        let Some(state) = &self.app.state else {
            return;
        };

        let current_cursor = match state.cursor_type.lock() {
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
