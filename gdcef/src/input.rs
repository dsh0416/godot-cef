use cef::sys::cef_event_flags_t;
use cef::{ImplBrowserHost, KeyEvent, KeyEventType, MouseButtonType, MouseEvent};
use godot::classes::{
    InputEventKey, InputEventMouseButton, InputEventMouseMotion, InputEventPanGesture,
};
use godot::global::{Key, MouseButton, MouseButtonMask};
use godot::prelude::*;

mod keycode;

/// Macro to extract keyboard modifier flags from any event with modifier methods
macro_rules! keyboard_modifiers {
    ($event:expr) => {{
        let mut modifiers = cef_event_flags_t::EVENTFLAG_NONE;
        if $event.is_shift_pressed() {
            modifiers |= cef_event_flags_t::EVENTFLAG_SHIFT_DOWN;
        }
        if $event.is_ctrl_pressed() {
            modifiers |= cef_event_flags_t::EVENTFLAG_CONTROL_DOWN;
        }
        if $event.is_alt_pressed() {
            modifiers |= cef_event_flags_t::EVENTFLAG_ALT_DOWN;
        }
        if $event.is_meta_pressed() {
            modifiers |= cef_event_flags_t::EVENTFLAG_COMMAND_DOWN;
        }

        // cef_event_flags_t returns u32 on linux and macOS, but i32 on Windows,
        // so we need to cast to u32 to avoid type mismatch.
        #[cfg(target_os = "windows")]
        let ret = modifiers.0 as u32;
        #[cfg(not(target_os = "windows"))]
        let ret = modifiers.0;
        ret
    }};
}

/// Extracts mouse button modifier flags from a button mask
fn mouse_button_modifiers(button_mask: MouseButtonMask) -> u32 {
    let mut modifiers = cef_event_flags_t::EVENTFLAG_NONE;

    if button_mask.is_set(MouseButtonMask::LEFT) {
        modifiers |= cef_event_flags_t::EVENTFLAG_LEFT_MOUSE_BUTTON;
    }
    if button_mask.is_set(MouseButtonMask::MIDDLE) {
        modifiers |= cef_event_flags_t::EVENTFLAG_MIDDLE_MOUSE_BUTTON;
    }
    if button_mask.is_set(MouseButtonMask::RIGHT) {
        modifiers |= cef_event_flags_t::EVENTFLAG_RIGHT_MOUSE_BUTTON;
    }

    // cef_event_flags_t returns u32 on linux and macOS, but i32 on Windows,
    // so we need to cast to u32 to avoid type mismatch.
    #[cfg(target_os = "windows")]
    return modifiers.0 as u32;
    #[cfg(not(target_os = "windows"))]
    return modifiers.0;
}

/// Creates a CEF mouse event from Godot position and DPI scale
pub fn create_mouse_event(
    position: Vector2,
    pixel_scale_factor: f32,
    device_scale_factor: f32,
    modifiers: i32,
) -> MouseEvent {
    let x = (position.x * pixel_scale_factor / device_scale_factor) as i32;
    let y = (position.y * pixel_scale_factor / device_scale_factor) as i32;

    MouseEvent {
        x,
        y,
        modifiers: modifiers as u32,
    }
}

/// Handles mouse button events and sends them to CEF browser host
pub fn handle_mouse_button(
    host: &impl ImplBrowserHost,
    event: &Gd<InputEventMouseButton>,
    pixel_scale_factor: f32,
    device_scale_factor: f32,
) {
    let modifiers =
        (keyboard_modifiers!(event) | mouse_button_modifiers(event.get_button_mask())) as i32;
    let position = event.get_position();
    let mouse_event =
        create_mouse_event(position, pixel_scale_factor, device_scale_factor, modifiers);

    match event.get_button_index() {
        MouseButton::LEFT | MouseButton::MIDDLE | MouseButton::RIGHT => {
            let button_type = match event.get_button_index() {
                MouseButton::LEFT => MouseButtonType::LEFT,
                MouseButton::MIDDLE => MouseButtonType::MIDDLE,
                MouseButton::RIGHT => MouseButtonType::RIGHT,
                _ => unreachable!(),
            };
            let mouse_up = !event.is_pressed();
            let click_count = if event.is_double_click() { 2 } else { 1 };
            host.send_mouse_click_event(
                Some(&mouse_event),
                button_type,
                mouse_up as i32,
                click_count,
            );
        }
        MouseButton::WHEEL_UP => {
            let delta = (120.0 * event.get_factor()) as i32;
            host.send_mouse_wheel_event(Some(&mouse_event), 0, delta);
        }
        MouseButton::WHEEL_DOWN => {
            let delta = (120.0 * event.get_factor()) as i32;
            host.send_mouse_wheel_event(Some(&mouse_event), 0, -delta);
        }
        MouseButton::WHEEL_LEFT => {
            let delta = (120.0 * event.get_factor()) as i32;
            host.send_mouse_wheel_event(Some(&mouse_event), -delta, 0);
        }
        MouseButton::WHEEL_RIGHT => {
            let delta = (120.0 * event.get_factor()) as i32;
            host.send_mouse_wheel_event(Some(&mouse_event), delta, 0);
        }
        _ => {}
    }
}

/// Handles mouse motion events and sends them to CEF browser host
pub fn handle_mouse_motion(
    host: &impl ImplBrowserHost,
    event: &Gd<InputEventMouseMotion>,
    pixel_scale_factor: f32,
    device_scale_factor: f32,
) {
    let modifiers = keyboard_modifiers!(event) | mouse_button_modifiers(event.get_button_mask());
    let position = event.get_position();
    let mouse_event = create_mouse_event(
        position,
        pixel_scale_factor,
        device_scale_factor,
        modifiers as i32,
    );
    host.send_mouse_move_event(Some(&mouse_event), false as i32);
}

/// Handles pan gesture events (trackpad scrolling) and sends them to CEF browser host
pub fn handle_pan_gesture(
    host: &impl ImplBrowserHost,
    event: &Gd<InputEventPanGesture>,
    pixel_scale_factor: f32,
    device_scale_factor: f32,
) {
    let modifiers = keyboard_modifiers!(event);
    let position = event.get_position();
    let mouse_event = create_mouse_event(
        position,
        pixel_scale_factor,
        device_scale_factor,
        modifiers as i32,
    );

    let delta = event.get_delta();
    // Convert pan delta to scroll wheel delta
    // Pan gesture delta is typically smaller, so we scale it up
    // Negative because pan direction is opposite to scroll direction
    let delta_x = (-delta.x * 120.0 / device_scale_factor) as i32;
    let delta_y = (-delta.y * 120.0 / device_scale_factor) as i32;

    if delta_x != 0 || delta_y != 0 {
        host.send_mouse_wheel_event(Some(&mouse_event), delta_x, delta_y);
    }
}

/// Handles keyboard events and sends them to CEF browser host
pub fn handle_key_event(host: &impl ImplBrowserHost, event: &Gd<InputEventKey>) {
    let mut modifiers = keyboard_modifiers!(event);
    #[cfg(target_os = "windows")]
    let keypad_key_modifier = cef_event_flags_t::EVENTFLAG_IS_KEY_PAD.0 as u32;
    #[cfg(not(target_os = "windows"))]
    let keypad_key_modifier = cef_event_flags_t::EVENTFLAG_IS_KEY_PAD.0;

    // Check if it's from the keypad
    if is_keypad_key(event.get_physical_keycode()) {
        modifiers |= keypad_key_modifier;
    }

    let is_pressed = event.is_pressed();
    let is_echo = event.is_echo();
    let keycode = event.get_keycode();

    // Get the Windows virtual key code from Godot key (CEF expects this on all platforms)
    let windows_key_code = keycode::godot_key_to_windows_keycode(keycode);

    // Get platform-specific native key code
    let native_key_code = keycode::godot_key_to_native_keycode(keycode);

    // Get the character code - for printable keys use unicode,
    // for control characters use their ASCII codes
    let unicode = event.get_unicode();
    let character = if unicode != 0 {
        unicode as u16
    } else {
        // Use ASCII codes for control characters
        get_control_char_code(keycode)
    };

    // For key press events (not echo/repeat), send RAWKEYDOWN
    if is_pressed && !is_echo {
        let key_event = KeyEvent {
            type_: KeyEventType::RAWKEYDOWN,
            modifiers,
            windows_key_code,
            native_key_code,
            is_system_key: 0,
            character,
            unmodified_character: character,
            focus_on_editable_field: 0,
            ..Default::default()
        };
        host.send_key_event(Some(&key_event));

        // Send a CHAR event for printable characters AND control characters that need it
        // (Backspace, Tab, Enter need CHAR events for text input to work)
        if should_send_char_event(keycode, unicode) {
            let char_event = KeyEvent {
                type_: KeyEventType::CHAR,
                modifiers,
                windows_key_code,
                native_key_code,
                is_system_key: 0,
                character,
                unmodified_character: character,
                focus_on_editable_field: 0,
                ..Default::default()
            };
            host.send_key_event(Some(&char_event));
        }
    } else if !is_pressed {
        // Key release event
        let key_event = KeyEvent {
            type_: KeyEventType::KEYUP,
            modifiers,
            windows_key_code,
            native_key_code,
            is_system_key: 0,
            character,
            unmodified_character: character,
            focus_on_editable_field: 0,
            ..Default::default()
        };
        host.send_key_event(Some(&key_event));
    }
}

/// Returns the ASCII control character code for special keys
fn get_control_char_code(key: Key) -> u16 {
    match key {
        Key::BACKSPACE => 0x08,             // BS (Backspace)
        Key::TAB => 0x09,                   // HT (Horizontal Tab)
        Key::ENTER | Key::KP_ENTER => 0x0D, // CR (Carriage Return)
        Key::ESCAPE => 0x1B,                // ESC
        Key::DELETE => 0x7F,                // DEL
        _ => 0,
    }
}

/// Determines if a CHAR event should be sent for this key
fn should_send_char_event(key: Key, unicode: u32) -> bool {
    // Send CHAR for printable characters
    if unicode != 0 && !is_modifier_key(key) {
        return true;
    }

    // Also send CHAR for control characters that text fields need
    matches!(
        key,
        Key::BACKSPACE | Key::TAB | Key::ENTER | Key::KP_ENTER | Key::DELETE
    )
}

/// Checks if a key is a modifier key (these should never send CHAR events)
fn is_modifier_key(key: Key) -> bool {
    matches!(
        key,
        Key::SHIFT
            | Key::CTRL
            | Key::ALT
            | Key::META
            | Key::CAPSLOCK
            | Key::NUMLOCK
            | Key::SCROLLLOCK
    )
}

/// Checks if a key is from the numeric keypad
fn is_keypad_key(key: Key) -> bool {
    matches!(
        key,
        Key::KP_0
            | Key::KP_1
            | Key::KP_2
            | Key::KP_3
            | Key::KP_4
            | Key::KP_5
            | Key::KP_6
            | Key::KP_7
            | Key::KP_8
            | Key::KP_9
            | Key::KP_MULTIPLY
            | Key::KP_SUBTRACT
            | Key::KP_PERIOD
            | Key::KP_ADD
            | Key::KP_DIVIDE
            | Key::KP_ENTER
    )
}

/// Commits IME text to the CEF browser
/// Call this when an IME composition is finalized
pub fn ime_commit_text(host: &impl ImplBrowserHost, text: &str) {
    let cef_text: cef::CefString = text.into();
    host.ime_commit_text(Some(&cef_text), None, 0);
}

/// Sets the current IME composition text
/// Call this during IME composition (before finalizing)
pub fn ime_set_composition(host: &impl ImplBrowserHost, text: &str) {
    let cef_text: cef::CefString = text.into();
    host.ime_set_composition(Some(&cef_text), None, None, None);
}

/// Cancels the current IME composition
pub fn ime_cancel_composition(host: &impl ImplBrowserHost) {
    host.ime_cancel_composition();
}

/// Finishes the current IME composition, committing the text
pub fn ime_finish_composing_text(host: &impl ImplBrowserHost, keep_selection: bool) {
    host.ime_finish_composing_text(keep_selection as i32);
}
