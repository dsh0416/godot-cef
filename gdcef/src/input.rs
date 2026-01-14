use cef::sys::cef_event_flags_t;
use cef::{ImplBrowserHost, ImplFrame, KeyEvent, KeyEventType, MouseButtonType, MouseEvent};
use godot::classes::{
    InputEvent, InputEventKey, InputEventMouseButton, InputEventMouseMotion, InputEventPanGesture,
};
use godot::global::{Key, MouseButton, MouseButtonMask};
use godot::prelude::*;

mod keycode;

/// Pre-defined shortcuts for editor commands.
/// Initialized once per thread using thread_local.
struct EditorShortcuts {
    select_all: Gd<InputEvent>,            // Ctrl/Cmd+A
    copy: Gd<InputEvent>,                  // Ctrl/Cmd+C
    cut: Gd<InputEvent>,                   // Ctrl/Cmd+X
    paste_and_match_style: Gd<InputEvent>, // Ctrl/Cmd+Shift+V
}

impl EditorShortcuts {
    fn new() -> Self {
        Self {
            select_all: create_shortcut(Key::A, true, false),
            copy: create_shortcut(Key::C, true, false),
            cut: create_shortcut(Key::X, true, false),
            paste_and_match_style: create_shortcut(Key::V, true, true),
        }
    }
}

fn with_shortcuts<F, R>(f: F) -> R
where
    F: FnOnce(&EditorShortcuts) -> R,
{
    let shortcuts = EditorShortcuts::new();
    f(&shortcuts)
}
fn create_shortcut(key: Key, with_command_or_ctrl: bool, with_shift: bool) -> Gd<InputEvent> {
    let mut key_event = InputEventKey::new_gd();
    key_event.set_keycode(key);

    if with_command_or_ctrl {
        key_event.set_command_or_control_autoremap(true);
    }

    if with_shift {
        key_event.set_shift_pressed(true);
    }

    key_event.to_variant().to()
}

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
pub fn handle_key_event(
    host: &impl ImplBrowserHost,
    frame: Option<&impl ImplFrame>,
    event: &Gd<InputEventKey>,
    focus_on_editable_field: bool,
) {
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

    // Godot also sends a KEY event for the NONE key for characters, which we don't want to process.
    if keycode == Key::NONE {
        return;
    }

    // Handle shortcuts using pre-cached Shortcut objects
    if is_pressed
        && !is_echo
        && let Some(frame) = frame
    {
        let input_event: Gd<InputEvent> = event.to_variant().to();
        let handled = with_shortcuts(|shortcuts| {
            if shortcuts.select_all.is_match(&input_event) {
                frame.select_all();
                return true;
            } else if shortcuts.copy.is_match(&input_event) {
                frame.copy();
                return true;
            } else if shortcuts.cut.is_match(&input_event) {
                frame.cut();
                return true;
            } else if shortcuts.paste_and_match_style.is_match(&input_event) {
                frame.paste_and_match_style();
                return true;
            }
            // The normal paste shortcut is handled by the browser host itself,
            // which is why we don't need to handle it here.
            false
        });
        if handled {
            return;
        }
    }

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

    // For key press events, send RAWKEYDOWN for initial press, KEYDOWN for repeat
    if is_pressed {
        let key_event = KeyEvent {
            type_: if is_echo {
                KeyEventType::KEYDOWN
            } else {
                KeyEventType::RAWKEYDOWN
            },
            modifiers,
            windows_key_code,
            native_key_code,
            is_system_key: 0,
            character,
            unmodified_character: character,
            focus_on_editable_field: focus_on_editable_field as _,
            ..Default::default()
        };
        host.send_key_event(Some(&key_event));

        // Send a CHAR event for printable characters AND control characters that need it
        // (Backspace, Tab, Enter need CHAR events for text input to work)
        // When focus is on an editable field, we don't need to send CHAR events.
        if should_send_char_event(keycode, unicode) && !focus_on_editable_field {
            let char_event = KeyEvent {
                type_: KeyEventType::CHAR,
                modifiers,
                // For CHAR events, use the character code (not the virtual key code)
                // for windows_key_code and native_key_code, matching Windows WM_CHAR
                // behavior where wParam contains the character value.
                windows_key_code: character as i32,
                native_key_code: character as i32,
                is_system_key: 0,
                character,
                unmodified_character: character,
                focus_on_editable_field: focus_on_editable_field as _,
                ..Default::default()
            };
            host.send_key_event(Some(&char_event));
        }
    } else {
        // Key release event
        // Skip KEYUP for navigation keys - works around a CEF issue where KEYUP
        // triggers arrow key actions on macOS
        if !is_navigation_key(keycode) {
            let key_event = KeyEvent {
                type_: KeyEventType::KEYUP,
                modifiers,
                windows_key_code,
                native_key_code,
                is_system_key: 0,
                character,
                unmodified_character: character,
                focus_on_editable_field: focus_on_editable_field as _,
                ..Default::default()
            };
            host.send_key_event(Some(&key_event));
        }
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
    // Never send CHAR for modifier or navigation keys
    if is_modifier_key(key) || is_navigation_key(key) {
        return false;
    }

    // Send CHAR for printable characters
    if unicode != 0 {
        return true;
    }

    false
}

/// Checks if a key is a navigation key (arrows, home, end, page up/down)
fn is_navigation_key(key: Key) -> bool {
    matches!(
        key,
        Key::UP
            | Key::DOWN
            | Key::LEFT
            | Key::RIGHT
            | Key::HOME
            | Key::END
            | Key::PAGEUP
            | Key::PAGEDOWN
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
    let invalid_range = cef::Range {
        from: u32::MAX,
        to: u32::MAX,
    };
    host.ime_commit_text(Some(&cef_text), Some(&invalid_range), 0);
}

pub fn ime_set_composition(
    host: &impl ImplBrowserHost,
    text: &str,
    selection_start: u32,
    selection_end: u32,
) {
    let cef_text: cef::CefString = text.into();
    let text_len = text.chars().count() as u32;

    // Create an underline for the entire composition text
    let underline = cef::CompositionUnderline {
        size: std::mem::size_of::<cef::CompositionUnderline>(),
        range: cef::Range {
            from: 0,
            to: text_len,
        },
        // Use default/system IME underline color (0 lets CEF choose an appropriate color)
        color: 0,
        background_color: 0,
        thick: 0, // thin underline
        style: cef::CompositionUnderlineStyle::SOLID,
    };
    let underlines = [underline];

    let invalid_range = cef::Range {
        from: u32::MAX,
        to: u32::MAX,
    };

    // Selection range is at the cursor position
    let selection_range = cef::Range {
        from: selection_start,
        to: selection_end,
    };

    host.ime_set_composition(
        Some(&cef_text),
        Some(&underlines),
        Some(&invalid_range),
        Some(&selection_range),
    );
}
