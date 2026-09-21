//! Godot selects the pointer target and provides copied, local GUI events.

use super::CefTexture;
use super::focus_state::FocusOwner;
use cef::ImplBrowserHost;
use godot::classes::{
    InputEvent, InputEventKey, InputEventMagnifyGesture, InputEventMouseButton,
    InputEventMouseMotion, InputEventPanGesture, InputEventScreenDrag, InputEventScreenTouch,
};
use godot::global::{Key, MouseButton};
use godot::prelude::*;

pub(super) fn browser_has_point(point: Vector2, size: Vector2, popup: Option<Rect2>) -> bool {
    Rect2::new(Vector2::ZERO, size).contains_point(point)
        || popup.is_some_and(|rect| rect.contains_point(point))
}

impl CefTexture {
    pub(super) fn handle_input_event(&mut self, event: Gd<InputEvent>) {
        let Ok(key) = event.try_cast::<InputEventKey>() else {
            return;
        };
        // Escape also cancels a source drag whose Godot adapter took focus.
        if key.is_pressed() && key.get_keycode() == Key::ESCAPE {
            self.cancel_browser_drag();
        }
        if !self.browser_input_available() || self.browser_focus_owner() == FocusOwner::Outside {
            return;
        }
        self.reconcile_browser_focus();
        let deliver_to_proxy = self.ime_active
            && crate::input::should_deliver_key_to_ime_proxy(
                key.get_keycode(),
                key.is_ctrl_pressed(),
                key.is_alt_pressed(),
                key.is_meta_pressed(),
            );
        self.texture2d_helper
            .bind()
            .forward_key_event(key, self.ime_active);
        // The proxy must receive normal Godot key/IME processing to produce
        // text_changed. The browser control has no such native text handler.
        if !deliver_to_proxy && let Some(mut viewport) = self.base().get_viewport() {
            viewport.set_input_as_handled();
        }
    }

    pub(super) fn handle_gui_input_event(&mut self, event: Gd<InputEvent>) {
        if !self.browser_input_available() {
            return;
        }
        let pixel_scale = self.get_pixel_scale_factor();
        let device_scale = self.get_device_scale_factor();

        // Unlike _input, _gui_input is hit-tested, respects mouse_filter and
        // follows Godot's mouse/touch capture. Positions already include canvas,
        // rotation and scale transforms. Never mutate the incoming event.
        if let Ok(button) = event.clone().try_cast::<InputEventMouseButton>() {
            self.last_pointer_position = button.get_position();
            let mask = match button.get_button_index() {
                MouseButton::LEFT => 1,
                MouseButton::RIGHT => 2,
                MouseButton::MIDDLE => 4,
                _ => 0,
            };
            let leave = self.pointer_state.button(mask, button.is_pressed());
            // Godot focuses the control before delivering a click. Apply CEF
            // focus before forwarding it without publishing an internal blur.
            self.reconcile_browser_focus();
            self.texture2d_helper.bind().forward_mouse_button_event(
                button,
                pixel_scale,
                device_scale,
            );
            if leave {
                self.send_browser_mouse_leave();
            }
        } else if let Ok(motion) = event.clone().try_cast::<InputEventMouseMotion>() {
            self.last_pointer_position = motion.get_position();
            self.pointer_state.motion();
            self.texture2d_helper.bind().forward_mouse_motion_event(
                motion,
                pixel_scale,
                device_scale,
            );
        } else if let Ok(pan) = event.clone().try_cast::<InputEventPanGesture>() {
            self.texture2d_helper
                .bind()
                .forward_pan_gesture_event(pan, pixel_scale, device_scale);
        } else if let Ok(touch) = event.clone().try_cast::<InputEventScreenTouch>() {
            if touch.is_pressed() {
                self.base_mut().grab_focus();
                self.reconcile_browser_focus();
            }
            self.texture2d_helper.bind_mut().forward_screen_touch_event(
                touch,
                pixel_scale,
                device_scale,
            );
        } else if let Ok(drag) = event.clone().try_cast::<InputEventScreenDrag>() {
            self.texture2d_helper.bind_mut().forward_screen_drag_event(
                drag,
                pixel_scale,
                device_scale,
            );
        } else if let Ok(magnify) = event.try_cast::<InputEventMagnifyGesture>() {
            self.texture2d_helper
                .bind()
                .forward_magnify_gesture_event(magnify);
        } else {
            // Keys were already routed through _input, including proxy input.
            return;
        }
        self.base_mut().accept_event();
    }

    pub(super) fn handle_browser_mouse_exit(&mut self) {
        if self.pointer_state.exit() {
            self.send_browser_mouse_leave();
        }
    }

    fn send_browser_mouse_leave(&self) {
        if let Some(host) = self.with_app(|app| app.host()) {
            let event = crate::input::create_mouse_event(
                self.last_pointer_position,
                self.get_pixel_scale_factor(),
                self.get_device_scale_factor(),
                0,
            );
            host.send_mouse_move_event(Some(&event), 1);
        }
    }

    pub(super) fn suspend_browser_input(&mut self) {
        self.suspend_browser_focus();
        self.texture2d_helper.bind_mut().cancel_active_touches();
        let had_mouse = self.pointer_state.has_mouse();
        let captured = self.pointer_state.reset();
        if captured && let Some(host) = self.with_app(|app| app.host()) {
            host.send_capture_lost_event();
        }
        if had_mouse {
            self.send_browser_mouse_leave();
        }
        self.cancel_browser_drag();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_extends_the_gui_target_only_while_visible() {
        let size = Vector2::new(100.0, 100.0);
        let popup = Rect2::new(Vector2::new(10.0, 90.0), Vector2::new(60.0, 80.0));
        assert!(browser_has_point(Vector2::new(50.0, 50.0), size, None));
        assert!(!browser_has_point(Vector2::new(20.0, 120.0), size, None));
        assert!(browser_has_point(
            Vector2::new(20.0, 120.0),
            size,
            Some(popup)
        ));
        assert!(!browser_has_point(
            Vector2::new(90.0, 120.0),
            size,
            Some(popup)
        ));
    }
}
