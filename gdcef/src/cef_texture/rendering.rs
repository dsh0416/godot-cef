use super::CefTexture;
use cef::{ImplBrowser, ImplBrowserHost};
use godot::classes::control::MouseFilter;
use godot::classes::image::Format as ImageFormat;
use godot::classes::texture_rect::ExpandMode;
use godot::classes::{DisplayServer, Engine, Image, TextureRect};
use godot::prelude::*;

use crate::browser::RenderMode;
use crate::utils::get_display_scale_factor;
use crate::{cursor, render};

/// Composite a popup buffer onto the main buffer at the specified position.
/// Both buffers are RGBA format.
fn composite_popup(
    dst: &mut [u8],
    dst_width: u32,
    dst_height: u32,
    src: &[u8],
    src_width: u32,
    src_height: u32,
    x: i32,
    y: i32,
) {
    // Clamp popup position to valid range
    let start_x = x.max(0) as u32;
    let start_y = y.max(0) as u32;

    // Calculate how much of the popup is visible
    let skip_x = if x < 0 { (-x) as u32 } else { 0 };
    let skip_y = if y < 0 { (-y) as u32 } else { 0 };

    let visible_width = (src_width.saturating_sub(skip_x)).min(dst_width.saturating_sub(start_x));
    let visible_height = (src_height.saturating_sub(skip_y)).min(dst_height.saturating_sub(start_y));

    if visible_width == 0 || visible_height == 0 {
        return;
    }

    // Copy popup pixels onto the main buffer
    for row in 0..visible_height {
        let src_row = skip_y + row;
        let dst_row = start_y + row;

        let src_row_start = ((src_row * src_width + skip_x) * 4) as usize;
        let dst_row_start = ((dst_row * dst_width + start_x) * 4) as usize;

        let copy_bytes = (visible_width * 4) as usize;

        if src_row_start + copy_bytes <= src.len() && dst_row_start + copy_bytes <= dst.len() {
            dst[dst_row_start..dst_row_start + copy_bytes]
                .copy_from_slice(&src[src_row_start..src_row_start + copy_bytes]);
        }
    }
}

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

            // Check popup state - we need to composite whenever popup is visible,
            // not just when it's dirty (to ensure it persists across frames)
            let popup_info = self.app.popup_state.as_ref().and_then(|ps| {
                ps.lock().ok().and_then(|popup| {
                    if popup.visible && !popup.buffer.is_empty() {
                        Some((
                            popup.buffer.clone(),
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

            let popup_visible = popup_info.is_some();
            let popup_dirty = popup_info.as_ref().is_some_and(|(_, _, _, _, _, dirty)| *dirty);

            // We need to update the texture if:
            // - The main frame buffer changed (fb.dirty)
            // - OR the popup changed (popup_dirty)
            if !fb.dirty && !popup_dirty {
                return;
            }

            if fb.data.is_empty() {
                return;
            }

            let width = fb.width as i32;
            let height = fb.height as i32;

            // Popup rect from CEF is in view coordinates (logical pixels).
            // The frame buffer is in physical pixels.
            // Scale popup position by display scale factor (same as what we report to CEF via screen_info).
            let display_scale = get_display_scale_factor();

            // Composite popup onto main buffer if visible
            let final_data = if let Some((popup_buffer, popup_width, popup_height, popup_x, popup_y, _)) = popup_info {
                let mut composited = fb.data.clone();
                let scaled_x = (popup_x as f32 * display_scale) as i32;
                let scaled_y = (popup_y as f32 * display_scale) as i32;
                composite_popup(
                    &mut composited,
                    fb.width,
                    fb.height,
                    &popup_buffer,
                    popup_width,
                    popup_height,
                    scaled_x,
                    scaled_y,
                );
                // Mark popup as clean (we've consumed its dirty state)
                if let Some(ps) = &self.app.popup_state {
                    if let Ok(mut popup) = ps.lock() {
                        popup.mark_clean();
                    }
                }
                composited
            } else if popup_visible {
                // Popup is visible but we couldn't get its data - shouldn't happen
                fb.data.clone()
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

        // Update popup overlay for accelerated rendering
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        if let Some(RenderMode::Accelerated { render_state, .. }) = &self.app.render_mode {
            // Check if we need to create a popup texture
            if let Ok(mut state) = render_state.lock() {
                if let Some((new_w, new_h)) = state.needs_popup_texture.take() {
                    // Free old popup texture if it exists
                    if let Some(old_rid) = state.popup_rd_rid {
                        render::free_rd_texture(old_rid);
                    }
                    
                    // Create new popup RD texture
                    match render::create_rd_texture(new_w as i32, new_h as i32) {
                        Ok((new_rid, new_texture_2d_rd)) => {
                            state.popup_rd_rid = Some(new_rid);
                            state.popup_width = new_w;
                            state.popup_height = new_h;
                            self.popup_texture_2d_rd = Some(new_texture_2d_rd);
                        }
                        Err(e) => {
                            godot::global::godot_error!("[CefTexture] Failed to create popup texture: {}", e);
                        }
                    }
                }
            }
            
            self.update_popup_overlay();
        }
    }

    /// Updates the popup overlay for accelerated rendering.
    /// In accelerated mode, the main view is rendered to a GPU texture,
    /// so we use a separate TextureRect overlay for the popup.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn update_popup_overlay(&mut self) {
        // Get popup visibility and position from popup_state
        let popup_visible_info = self.app.popup_state.as_ref().and_then(|ps| {
            ps.lock().ok().and_then(|popup| {
                if popup.visible {
                    Some((popup.rect.x, popup.rect.y, popup.rect.width, popup.rect.height))
                } else {
                    None
                }
            })
        });
        
        // Get accelerated render state info (popup_dirty, dimensions)
        let accel_popup_info = if let Some(RenderMode::Accelerated { render_state, .. }) = &self.app.render_mode {
            render_state.lock().ok().and_then(|state| {
                if state.popup_rd_rid.is_some() && state.popup_width > 0 && state.popup_height > 0 {
                    Some((state.popup_dirty, state.popup_width, state.popup_height))
                } else {
                    None
                }
            })
        } else {
            None
        };

        match (popup_visible_info, accel_popup_info) {
            (Some((x, y, _rect_w, _rect_h)), Some((popup_dirty, tex_width, tex_height))) => {
                // Create overlay TextureRect if it doesn't exist
                if self.popup_overlay.is_none() {
                    let mut overlay = TextureRect::new_alloc();
                    overlay.set_expand_mode(ExpandMode::IGNORE_SIZE);
                    overlay.set_mouse_filter(MouseFilter::IGNORE);
                    let overlay_node: Gd<godot::classes::Node> = overlay.clone().upcast();
                    self.base_mut().add_child(&overlay_node);
                    self.popup_overlay = Some(overlay);
                }

                // Get CefTexture size and render size for coordinate mapping
                let display_scale = get_display_scale_factor();
                let cef_texture_size = self.base().get_size();
                
                // Get the render size that was passed to CEF
                let render_size = self.app.render_size.as_ref().map(|s| {
                    s.lock().ok().map(|sz| (sz.width, sz.height))
                }).flatten().unwrap_or((0.0, 0.0));

                // Set the Texture2DRD on the overlay (GPU texture)
                if let Some(overlay) = &mut self.popup_overlay {
                    if let Some(texture_2d_rd) = &self.popup_texture_2d_rd {
                        overlay.set_texture(texture_2d_rd);
                    }
                    
                    // CEF's view_rect reports (render_size / display_scale) to CEF as the DIP size.
                    // Popup coordinates are in that DIP space. To map to CefTexture's coordinate
                    // space, we need to scale by: cef_texture_size / DIP_size
                    // = cef_texture_size / (render_size / display_scale)
                    // = cef_texture_size * display_scale / render_size
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
                    // Texture size needs similar scaling
                    let local_width = tex_width as f32 * scale_x / display_scale;
                    let local_height = tex_height as f32 * scale_y / display_scale;

                    overlay.set_position(Vector2::new(local_x, local_y));
                    overlay.set_size(Vector2::new(local_width, local_height));
                    overlay.set_visible(true);
                }

                // Mark popup as clean in accelerated render state
                if popup_dirty {
                    if let Some(RenderMode::Accelerated { render_state, .. }) = &self.app.render_mode {
                        if let Ok(mut state) = render_state.lock() {
                            state.popup_dirty = false;
                        }
                    }
                }
            }
            _ => {
                // Hide popup overlay when popup is not visible
                if let Some(overlay) = &mut self.popup_overlay {
                    overlay.set_visible(false);
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
