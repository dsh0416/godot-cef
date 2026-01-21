//! IME (Input Method Editor) handling for CefTexture.
//!
//! This module contains methods for IME composition, proxy management,
//! and cursor positioning.

use super::CefTexture;
use cef::{ImplBrowser, ImplBrowserHost};
use godot::classes::control::{FocusMode, MouseFilter};
use godot::classes::{Control, DisplayServer, LineEdit};
use godot::prelude::*;

use crate::input;
use crate::utils::{get_display_scale_factor, try_lock};

impl CefTexture {
    /// Creates a hidden LineEdit to act as an IME input proxy.
    pub(super) fn create_ime_proxy(&mut self) {
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
        self.ime_proxy = Some(line_edit);
    }

    pub(super) fn process_ime_enable_queue(&mut self) {
        let Some(queue) = &self.app.ime_enable_queue else {
            return;
        };

        let final_req: Option<bool> = {
            let Some(mut q) = try_lock!(queue, "ime_enable_queue") else {
                return;
            };
            q.drain(..).next_back()
        };

        if let Some(enable) = final_req {
            if enable && !self.ime_active {
                self.activate_ime();
            } else if !enable && self.ime_active {
                self.deactivate_ime();
            }
        }
    }

    pub(super) fn process_ime_composition_queue(&mut self) {
        let Some(queue) = &self.app.ime_composition_range else {
            return;
        };

        let range = {
            let Some(mut q) = try_lock!(queue, "ime_composition_queue") else {
                return;
            };
            q.take()
        };

        if let Some(range) = range
            && self.ime_active
        {
            // Directly assign to ime_position field instead of using setter
            // to avoid conflict with GodotClass-generated setter
            self.ime_position = Vector2i::new(range.caret_x, range.caret_y + range.caret_height);
            self.process_ime_position();
        }
    }

    pub(super) fn process_ime_position(&mut self) {
        if self.ime_active {
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

    /// Called when the IME proxy LineEdit text changes during composition.
    pub(super) fn on_ime_proxy_text_changed_impl(&mut self, new_text: GString) {
        let Some(browser) = self.app.browser.as_mut() else {
            return;
        };

        let Some(host) = browser.host() else {
            return;
        };

        input::ime_commit_text(&host, &new_text.to_string());

        if let Some(proxy) = self.ime_proxy.as_mut() {
            proxy.set_text("");
        }
    }

    pub(super) fn on_ime_proxy_focus_exited_impl(&mut self) {
        if self.ime_focus_regrab_pending {
            return;
        }

        // Defer the check to the next frame when the focus system has settled
        self.base_mut()
            .call_deferred("_check_ime_focus_after_exit", &[]);
    }

    pub(super) fn check_ime_focus_after_exit_impl(&mut self) {
        if !self.ime_active {
            return;
        }

        if let Some(viewport) = self.base().get_viewport()
            && let Some(focused) = viewport.gui_get_focus_owner()
        {
            let self_control = self.base().clone().upcast::<Control>();

            if focused == self_control {
                self.ime_focus_regrab_pending = true;
                self.base_mut().release_focus();
                if let Some(proxy) = self.ime_proxy.as_mut() {
                    proxy.grab_focus();
                }
                self.ime_focus_regrab_pending = false;
                return;
            }
        }

        self.deactivate_ime();
    }

    /// Activates IME by focusing the hidden LineEdit proxy.
    pub(super) fn activate_ime(&mut self) {
        if self.ime_active {
            return;
        }

        self.base_mut().release_focus();

        if let Some(proxy) = self.ime_proxy.as_mut() {
            proxy.set_text("");
            proxy.grab_focus();
        }

        if let Some(browser) = self.app.browser.as_mut()
            && let Some(host) = browser.host()
        {
            host.set_focus(true as _);
        }

        self.ime_active = true;
    }

    /// Deactivates IME and commits any pending text.
    pub(super) fn deactivate_ime(&mut self) {
        if !self.ime_active {
            return;
        }

        // Clear the proxy
        if let Some(proxy) = self.ime_proxy.as_mut() {
            proxy.set_text("");
        }

        self.ime_active = false;

        // Return focus to CefTexture
        self.base_mut().grab_focus();
    }

    pub(super) fn handle_os_ime_update(&mut self) {
        if !self.ime_active {
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
