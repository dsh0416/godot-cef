//! IME (Input Method Editor) handling for CefTexture.
//!
//! This module contains methods for IME composition, proxy management,
//! and cursor positioning.

use super::CefTexture;
use super::focus_state::FocusOwner;
use cef::ImplBrowserHost;
use godot::classes::control::{FocusMode, MouseFilter};
use godot::classes::{Control, DisplayServer, LineEdit};
use godot::prelude::*;

use crate::input;
use crate::utils::get_display_scale_factor;

impl CefTexture {
    /// Creates a hidden LineEdit to act as an IME input proxy.
    pub(super) fn create_ime_proxy(&mut self) {
        let mut line_edit = LineEdit::new_alloc();
        line_edit.set_position(Vector2::new(-10000.0, -10000.0));
        line_edit.set_size(Vector2::new(200.0, 30.0));
        line_edit.set_mouse_filter(MouseFilter::IGNORE);
        line_edit.set_focus_mode(FocusMode::ALL);
        // Keep the proxy in edit mode across `ui_text_submit`. The proxy is
        // hidden and never meant to submit anything, but the page's Enter is
        // forwarded to it, and LineEdit leaves edit mode on that action
        // (`unedit()`), after which `gui_input` drops every key event because of
        // its `if (!editing) return;` guard. The host would then report focus and
        // editability as healthy while no text could reach the page, until a
        // mouse click re-entered edit mode.
        line_edit.set_keep_editing_on_text_submit(true);
        let callable_changed = self.base().callable("on_ime_proxy_text_changed");
        line_edit.connect("text_changed", &callable_changed);

        let callable_focus_changed = self.base().callable("on_ime_proxy_focus_exited");
        line_edit.connect("focus_entered", &callable_focus_changed);
        line_edit.connect("focus_exited", &callable_focus_changed);

        self.base_mut().add_child(&line_edit);
        self.ime_proxy = Some(line_edit);
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
        if self.ime_active
            && self.browser_input_available()
            && self.browser_focus_owner() == FocusOwner::Proxy
            && let Some(host) = self.with_app(|app| app.host())
        {
            input::ime_commit_text(&host, &new_text.to_string());
        }

        if let Some(proxy) = self.ime_proxy.as_mut() {
            proxy.set_text("");
        }
    }

    pub(super) fn on_ime_proxy_focus_exited_impl(&mut self) {
        self.defer_browser_focus_update();
    }

    pub(super) fn defer_browser_focus_update(&mut self) {
        if self.focus_reconcile_pending {
            return;
        }
        self.focus_reconcile_pending = true;
        // Focus-exited is emitted before Godot assigns the new owner. Inspect
        // the settled owner, never the transient gap during a proxy transfer.
        self.base_mut()
            .call_deferred("_check_ime_focus_after_exit", &[]);
    }

    pub(super) fn check_ime_focus_after_exit_impl(&mut self) {
        self.focus_reconcile_pending = false;
        self.reconcile_browser_focus();
    }

    pub(super) fn browser_focus_owner(&self) -> FocusOwner {
        if !self.base().is_inside_tree() {
            return FocusOwner::Outside;
        }
        if let Some(viewport) = self.base().get_viewport()
            && let Some(focused) = viewport.gui_get_focus_owner()
        {
            let self_control = self.base().clone().upcast::<Control>();
            if focused == self_control {
                return FocusOwner::Browser;
            }
            if self
                .ime_proxy
                .as_ref()
                .is_some_and(|proxy| focused == proxy.clone().upcast::<Control>())
            {
                return FocusOwner::Proxy;
            }
        }
        FocusOwner::Outside
    }

    pub(super) fn browser_input_available(&self) -> bool {
        self.base().is_inside_tree()
            && self.base().is_visible_in_tree()
            && self.base().can_process()
            && self.base().is_processing()
            && self
                .base()
                .get_window()
                .is_some_and(|window| window.has_focus())
            && self.with_app(|app| app.state.is_some())
    }

    pub(super) fn reconcile_browser_focus(&mut self) {
        let owner = self.browser_focus_owner();
        let available = self.browser_input_available();
        self.apply_browser_focus(owner, available);
    }

    pub(super) fn suspend_browser_focus(&mut self) {
        self.apply_browser_focus(FocusOwner::Outside, false);
    }

    fn apply_browser_focus(&mut self, owner: FocusOwner, available: bool) {
        let Some(host) = self.with_app(|app| app.host()) else {
            // A future browser must receive focus even if the logical owner
            // has not changed since this node started processing.
            self.focus_state.invalidate_host_focus();
            self.ime_active = false;
            return;
        };
        let update = self.focus_state.reconcile(owner, available);
        // Clear composition before losing CEF focus, and never commit a late
        // proxy signal into whichever DOM element happens to be active later.
        if self.ime_active && !update.ime_active {
            host.ime_cancel_composition();
            if let Some(proxy) = self.ime_proxy.as_mut() {
                proxy.set_text("");
            }
        }
        self.ime_active = update.ime_active;

        if let Some(focused) = update.cef_focus {
            host.set_focus(focused as _);
        }

        match update.transfer {
            Some(FocusOwner::Proxy) => {
                // grab_focus performs the transfer. Do not separately release
                // the browser control and publish a spurious CEF blur.
                if let Some(mut proxy) = self.ime_proxy.clone() {
                    // This can synchronously notify the parent control. Keep
                    // its Rust borrow suspended while Godot transfers focus.
                    let _guard = self.base_mut();
                    proxy.grab_focus();
                }
            }
            Some(FocusOwner::Browser) => self.base_mut().grab_focus(),
            _ => {}
        }
    }

    pub(super) fn handle_os_ime_update(&mut self) {
        if !self.ime_active
            || !self.browser_input_available()
            || self.browser_focus_owner() != FocusOwner::Proxy
        {
            return;
        }

        let ime_text = DisplayServer::singleton().ime_get_text().to_string();
        let ime_selection = DisplayServer::singleton().ime_get_selection();
        let start = ime_selection.x.max(0) as u32;
        let end = ime_selection.y.max(0) as u32;

        // Update the IME composition text
        if let Some(host) = self.with_app(|app| app.host()) {
            input::ime_set_composition(&host, &ime_text, start, end);
        }
    }
}
