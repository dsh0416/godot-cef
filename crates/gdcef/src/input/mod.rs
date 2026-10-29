use cef::sys::cef_event_flags_t;
use cef::{
    ImplBrowserHost, ImplFrame, KeyEvent, KeyEventType, MouseButtonType, MouseEvent, PointerType,
    TouchEvent, TouchEventType,
};
use godot::classes::{
    InputEvent, InputEventKey, InputEventMagnifyGesture, InputEventMouseButton,
    InputEventMouseMotion, InputEventPanGesture, InputEventScreenDrag, InputEventScreenTouch,
};
use godot::global::{Key, MouseButton, MouseButtonMask};
#[cfg(target_os = "macos")]
use godot::init::GdextBuild;
use godot::prelude::*;
use std::collections::HashMap;

mod keycode;

/// Standard wheel delta for one scroll "notch" (Windows convention used by CEF).
const WHEEL_DELTA: f32 = 120.0;
const MAGNIFY_ZOOM_SENSITIVITY: f64 = 0.5;

/// Pre-defined shortcuts for editor commands.
///
/// Created on each use rather than cached in `thread_local!` because `Gd<T>`
/// objects require the Godot engine to be alive during `Drop`. Thread-local
/// destructors run after Godot has shut down, causing either a panic or leaked
/// instances. Since keyboard shortcuts are matched at human typing speed (not
/// per-frame), the allocation cost is negligible.
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

        crate::compat::event_flags_to_u32(modifiers)
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

    crate::compat::event_flags_to_u32(modifiers)
}

/// Creates a CEF mouse event from Godot position and DPI scale
pub fn create_mouse_event(
    position: Vector2,
    pixel_scale_factor: f32,
    device_scale_factor: f32,
    modifiers: u32,
) -> MouseEvent {
    let x = (position.x * pixel_scale_factor / device_scale_factor) as i32;
    let y = (position.y * pixel_scale_factor / device_scale_factor) as i32;

    MouseEvent { x, y, modifiers }
}

/// Handles mouse button events and sends them to CEF browser host
pub fn handle_mouse_button(
    host: &impl ImplBrowserHost,
    event: &Gd<InputEventMouseButton>,
    pixel_scale_factor: f32,
    device_scale_factor: f32,
) {
    let modifiers = keyboard_modifiers!(event) | mouse_button_modifiers(event.get_button_mask());
    let position = event.get_position();
    let mouse_event =
        create_mouse_event(position, pixel_scale_factor, device_scale_factor, modifiers);

    match event.get_button_index() {
        MouseButton::LEFT | MouseButton::MIDDLE | MouseButton::RIGHT => {
            let button_type = match event.get_button_index() {
                MouseButton::LEFT => MouseButtonType::LEFT,
                MouseButton::MIDDLE => MouseButtonType::MIDDLE,
                MouseButton::RIGHT => MouseButtonType::RIGHT,
                _ => return,
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
            let delta = (WHEEL_DELTA * event.get_factor()) as i32;
            host.send_mouse_wheel_event(Some(&mouse_event), 0, delta);
        }
        MouseButton::WHEEL_DOWN => {
            let delta = (WHEEL_DELTA * event.get_factor()) as i32;
            host.send_mouse_wheel_event(Some(&mouse_event), 0, -delta);
        }
        MouseButton::WHEEL_LEFT => {
            let delta = (WHEEL_DELTA * event.get_factor()) as i32;
            host.send_mouse_wheel_event(Some(&mouse_event), -delta, 0);
        }
        MouseButton::WHEEL_RIGHT => {
            let delta = (WHEEL_DELTA * event.get_factor()) as i32;
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
    let mouse_event =
        create_mouse_event(position, pixel_scale_factor, device_scale_factor, modifiers);
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
    let mouse_event =
        create_mouse_event(position, pixel_scale_factor, device_scale_factor, modifiers);

    let (delta_x, delta_y) = pan_gesture_wheel_delta(event.get_delta(), device_scale_factor);

    if delta_x != 0 || delta_y != 0 {
        host.send_mouse_wheel_event(Some(&mouse_event), delta_x, delta_y);
    }
}

/// Converts a pan gesture delta to a CEF wheel delta in browser pixels.
///
/// On macOS, Godot makes the pan delta from the `NSEvent` scrolling delta, which is in
/// points. Chromium scrolls a page by this delta, so CEF gets it without Godot's factor.
/// The screen scale does not apply, because points are already device-independent.
#[cfg(target_os = "macos")]
fn pan_gesture_wheel_delta(delta: Vector2, _device_scale_factor: f32) -> (i32, i32) {
    let (major, minor, _) = GdextBuild::godot_runtime_version_triple();
    macos_pan_wheel_delta(delta, macos_pan_delta_factor(major, minor))
}

/// Converts a pan gesture delta to a CEF wheel delta in browser pixels.
#[cfg(not(target_os = "macos"))]
fn pan_gesture_wheel_delta(delta: Vector2, device_scale_factor: f32) -> (i32, i32) {
    // Pan gesture delta is typically smaller, so we scale it up
    // Negative because pan direction is opposite to scroll direction
    let delta_x = (-delta.x * WHEEL_DELTA / device_scale_factor) as i32;
    let delta_y = (-delta.y * WHEEL_DELTA / device_scale_factor) as i32;
    (delta_x, delta_y)
}

/// Returns the factor that Godot applies to a precise macOS scrolling delta before it
/// sends a pan gesture. Godot 4.8 removed the factor:
/// <https://github.com/godotengine/godot/pull/123072>
#[cfg(any(target_os = "macos", test))]
fn macos_pan_delta_factor(godot_major: u8, godot_minor: u8) -> f32 {
    if (godot_major, godot_minor) < (4, 8) {
        0.03
    } else {
        1.0
    }
}

/// Converts a macOS pan gesture delta back to the `NSEvent` scrolling delta in points.
#[cfg(any(target_os = "macos", test))]
fn macos_pan_wheel_delta(delta: Vector2, pan_delta_factor: f32) -> (i32, i32) {
    // Negative because pan direction is opposite to scroll direction.
    // CEF takes whole pixels, and the scrolling delta can have a fraction, so round it.
    let delta_x = (-delta.x / pan_delta_factor).round() as i32;
    let delta_y = (-delta.y / pan_delta_factor).round() as i32;
    (delta_x, delta_y)
}

fn create_touch_event(
    position: Vector2,
    pixel_scale_factor: f32,
    device_scale_factor: f32,
    type_: TouchEventType,
    id: i32,
    pressure: f32,
) -> TouchEvent {
    TouchEvent {
        id,
        x: position.x * pixel_scale_factor / device_scale_factor,
        y: position.y * pixel_scale_factor / device_scale_factor,
        pressure: pressure.clamp(0.0, 1.0),
        type_,
        pointer_type: PointerType::TOUCH,
        ..Default::default()
    }
}

fn touch_id_for_event(
    touch_id_map: &mut HashMap<i32, i32>,
    index: i32,
    type_: TouchEventType,
) -> Option<i32> {
    if type_ == TouchEventType::PRESSED {
        if touch_id_map.contains_key(&index) {
            return None;
        }
        // Godot indices are stable for a touch. Track only accepted presses so
        // late moves/releases after a focus-loss cancellation are ignored.
        touch_id_map.insert(index, index);
        Some(index)
    } else if type_ == TouchEventType::MOVED {
        touch_id_map.get(&index).copied()
    } else {
        touch_id_map.remove(&index)
    }
}

fn cancelled_touch_events(touch_id_map: &mut HashMap<i32, i32>) -> Vec<TouchEvent> {
    touch_id_map
        .drain()
        .map(|(_, id)| TouchEvent {
            id,
            type_: TouchEventType::CANCELLED,
            pointer_type: PointerType::TOUCH,
            ..Default::default()
        })
        .collect()
}

pub(crate) fn cancel_screen_touches(
    host: &impl ImplBrowserHost,
    touch_id_map: &mut HashMap<i32, i32>,
) {
    for event in cancelled_touch_events(touch_id_map) {
        host.send_touch_event(Some(&event));
    }
}

/// Handles screen touch events and sends them to CEF browser host.
pub fn handle_screen_touch(
    host: &impl ImplBrowserHost,
    event: &Gd<InputEventScreenTouch>,
    pixel_scale_factor: f32,
    device_scale_factor: f32,
    touch_id_map: &mut HashMap<i32, i32>,
    _next_touch_id: &mut i32,
) {
    let index = event.get_index();
    let is_canceled = event.is_canceled();
    let is_pressed = event.is_pressed();
    let type_ = if is_canceled {
        TouchEventType::CANCELLED
    } else if is_pressed {
        TouchEventType::PRESSED
    } else {
        TouchEventType::RELEASED
    };

    let Some(id) = touch_id_for_event(touch_id_map, index, type_) else {
        return;
    };
    let touch_event = create_touch_event(
        event.get_position(),
        pixel_scale_factor,
        device_scale_factor,
        type_,
        id,
        1.0,
    );
    host.send_touch_event(Some(&touch_event));
}

/// Handles screen drag events and sends them to CEF browser host.
pub fn handle_screen_drag(
    host: &impl ImplBrowserHost,
    event: &Gd<InputEventScreenDrag>,
    pixel_scale_factor: f32,
    device_scale_factor: f32,
    touch_id_map: &mut HashMap<i32, i32>,
    _next_touch_id: &mut i32,
) {
    let index = event.get_index();
    let Some(id) = touch_id_for_event(touch_id_map, index, TouchEventType::MOVED) else {
        return;
    };
    let touch_event = create_touch_event(
        event.get_position(),
        pixel_scale_factor,
        device_scale_factor,
        TouchEventType::MOVED,
        id,
        event.get_pressure(),
    );
    host.send_touch_event(Some(&touch_event));
}

/// Handles magnify gesture events and maps them to browser zoom level changes.
pub fn handle_magnify_gesture(host: &impl ImplBrowserHost, event: &Gd<InputEventMagnifyGesture>) {
    let factor = event.get_factor() as f64;
    if factor.abs() <= f64::EPSILON {
        return;
    }

    let current_zoom = host.zoom_level();
    host.set_zoom_level(current_zoom + (factor * MAGNIFY_ZOOM_SENSITIVITY));
}

/// Keeps browser shortcuts out of the hidden native text proxy.
pub(crate) fn should_deliver_key_to_ime_proxy(
    keycode: Key,
    ctrl: bool,
    alt: bool,
    meta: bool,
) -> bool {
    // Tab belongs to the page's focus traversal. Editing shortcuts are also
    // browser-owned: allowing the proxy to paste would insert the text twice.
    // Ctrl+Alt is how some platforms expose AltGr and must still produce text.
    keycode != Key::TAB && !meta && (!ctrl || alt)
}

/// Handles keyboard events and sends them to CEF browser host.
pub fn handle_key_event(
    host: &impl ImplBrowserHost,
    frame: Option<&impl ImplFrame>,
    event: &Gd<InputEventKey>,
    focus_on_editable_field: bool,
) {
    let mut modifiers = keyboard_modifiers!(event);

    // Check if it's from the keypad
    if is_keypad_key(event.get_physical_keycode()) {
        modifiers |= crate::compat::event_flags_to_u32(cef_event_flags_t::EVENTFLAG_IS_KEY_PAD);
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

    #[cfg(target_os = "macos")]
    let platform = KeyboardPlatform::MacOS;
    #[cfg(not(target_os = "macos"))]
    let platform = KeyboardPlatform::Other;

    let input = KeyInput {
        keycode,
        #[cfg(any(target_os = "macos", test))]
        physical_keycode: event.get_physical_keycode(),
        unicode: event.get_unicode(),
        modifiers,
        is_pressed,
        is_echo,
        focus_on_editable_field,
    };
    for key_event in key_events(input, platform).into_iter().flatten() {
        host.send_key_event(Some(&key_event));
    }
}

#[derive(Clone, Copy)]
struct KeyInput {
    keycode: Key,
    #[cfg(any(target_os = "macos", test))]
    physical_keycode: Key,
    unicode: u32,
    modifiers: u32,
    is_pressed: bool,
    is_echo: bool,
    focus_on_editable_field: bool,
}

#[derive(Clone, Copy)]
enum KeyboardPlatform {
    #[cfg(any(target_os = "macos", test))]
    MacOS,
    #[cfg(not(target_os = "macos"))]
    Other,
}

/// Builds the events forwarded to CEF without requiring a running Godot instance.
fn key_events(input: KeyInput, platform: KeyboardPlatform) -> [Option<KeyEvent>; 2] {
    let windows_key_code = keycode::godot_key_to_windows_keycode(input.keycode);
    let (native_key_code, character) = match platform {
        #[cfg(any(target_os = "macos", test))]
        KeyboardPlatform::MacOS => {
            // Cocoa key codes describe physical positions, even when the layout
            // or Shift changes Godot's logical key code (e.g. 1 becomes !).
            let physical_keycode = if input.physical_keycode == Key::NONE {
                input.keycode
            } else {
                input.physical_keycode
            };
            (
                keycode::godot_key_to_macos_keycode(physical_keycode),
                macos_key_character(input.keycode, input.unicode, input.modifiers),
            )
        }
        #[cfg(not(target_os = "macos"))]
        KeyboardPlatform::Other => (
            keycode::godot_key_to_native_keycode(input.keycode),
            if input.unicode != 0 {
                input.unicode as u16
            } else {
                get_control_char_code(input.keycode)
            },
        ),
    };
    let key_event = KeyEvent {
        type_: if !input.is_pressed {
            KeyEventType::KEYUP
        } else if input.is_echo {
            KeyEventType::KEYDOWN
        } else {
            KeyEventType::RAWKEYDOWN
        },
        modifiers: input.modifiers,
        windows_key_code,
        native_key_code,
        character,
        unmodified_character: character,
        focus_on_editable_field: input.focus_on_editable_field as _,
        ..Default::default()
    };

    // Reconstructed characters identify the key to Cocoa; they must not turn a
    // zero-Unicode event into text input. The IME proxy owns editable text input.
    let char_event = if input.is_pressed
        && should_send_char_event(input.keycode, input.unicode)
        && !input.focus_on_editable_field
    {
        let mut char_event = key_event.clone();
        char_event.type_ = KeyEventType::CHAR;
        match platform {
            #[cfg(any(target_os = "macos", test))]
            KeyboardPlatform::MacOS => {
                // CEF constructs an NSEvent for CHAR too, so keep its hardware
                // key code instead of replacing it with the character value.
            }
            #[cfg(not(target_os = "macos"))]
            KeyboardPlatform::Other => {
                char_event.windows_key_code = character as i32;
                char_event.native_key_code = character as i32;
            }
        }
        Some(char_event)
    } else {
        None
    };
    [Some(key_event), char_event]
}

/// Supplies Cocoa's character identity when Godot omits Unicode, notably on keyup.
#[cfg(any(target_os = "macos", test))]
fn macos_key_character(key: Key, unicode: u32, modifiers: u32) -> u16 {
    if is_modifier_key(key) {
        return 0;
    }
    if unicode != 0 {
        return unicode as u16;
    }

    // CEF interprets two zero character fields as NSEventTypeFlagsChanged,
    // regardless of KEYUP. Chromium then treats non-modifier keys as keydown.
    // Use AppKit's function-key characters so keyup retains its original type.
    match key {
        Key::UP => 0xF700,
        Key::DOWN => 0xF701,
        Key::LEFT => 0xF702,
        Key::RIGHT => 0xF703,
        Key::INSERT => 0xF727,
        Key::DELETE => 0xF728,
        Key::HOME => 0xF729,
        Key::END => 0xF72B,
        Key::PAGEUP => 0xF72C,
        Key::PAGEDOWN => 0xF72D,
        Key::KP_MULTIPLY => b'*' as u16,
        Key::KP_ADD => b'+' as u16,
        Key::KP_SUBTRACT => b'-' as u16,
        Key::KP_PERIOD => b'.' as u16,
        Key::KP_DIVIDE => b'/' as u16,
        _ if (Key::F1.ord()..=Key::F35.ord()).contains(&key.ord()) => {
            (0xF704 + key.ord() - Key::F1.ord()) as u16
        }
        _ if (Key::KP_0.ord()..=Key::KP_9.ord()).contains(&key.ord()) => {
            (i32::from(b'0') + key.ord() - Key::KP_0.ord()) as u16
        }
        _ => {
            let control = get_control_char_code(key);
            if control != 0 {
                return control;
            }
            let Ok(character) = u16::try_from(key.ord()) else {
                return 0;
            };
            let shift = crate::compat::event_flags_to_u32(cef_event_flags_t::EVENTFLAG_SHIFT_DOWN);
            if (u16::from(b'A')..=u16::from(b'Z')).contains(&character) && modifiers & shift == 0 {
                character + u16::from(b'a' - b'A')
            } else {
                character
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn key_input(keycode: Key, unicode: u32, is_pressed: bool) -> KeyInput {
        KeyInput {
            keycode,
            physical_keycode: keycode,
            unicode,
            modifiers: 0,
            is_pressed,
            is_echo: false,
            focus_on_editable_field: false,
        }
    }

    #[test]
    fn macos_zero_unicode_releases_remain_keyup_in_cef() {
        for (key, character) in [
            (Key::A, b'a' as u16),
            (Key::Z, b'z' as u16),
            (Key::KEY_0, b'0' as u16),
            (Key::KEY_9, b'9' as u16),
            (Key::SPACE, b' ' as u16),
            (Key::QUOTELEFT, b'`' as u16),
            (Key::APOSTROPHE, b'\'' as u16),
            (Key::SEMICOLON, b';' as u16),
            (Key::KP_0, b'0' as u16),
            (Key::KP_9, b'9' as u16),
            (Key::KP_ADD, b'+' as u16),
            (Key::UP, 0xF700),
            (Key::DOWN, 0xF701),
            (Key::LEFT, 0xF702),
            (Key::RIGHT, 0xF703),
            (Key::HOME, 0xF729),
            (Key::END, 0xF72B),
            (Key::PAGEUP, 0xF72C),
            (Key::PAGEDOWN, 0xF72D),
            (Key::F1, 0xF704),
            (Key::F12, 0xF70F),
        ] {
            let events = key_events(key_input(key, 0, false), KeyboardPlatform::MacOS);
            assert_eq!(
                events[0].as_ref().map(|event| event.type_),
                Some(KeyEventType::KEYUP),
                "{key:?}"
            );
            // Both fields must identify a non-modifier NSEvent. Merely setting
            // KEYUP with zero characters makes CEF emit another raw keydown.
            assert_eq!(
                events[0]
                    .as_ref()
                    .map(|event| (event.character, event.unmodified_character)),
                Some((character, character)),
                "{key:?}"
            );
            assert!(events[1].is_none());
        }
    }

    #[test]
    fn macos_function_and_navigation_presses_do_not_generate_text() {
        for key in [Key::LEFT, Key::HOME, Key::PAGEUP, Key::F1, Key::F12] {
            let events = key_events(key_input(key, 0, true), KeyboardPlatform::MacOS);
            assert_eq!(
                events[0].as_ref().map(|event| event.type_),
                Some(KeyEventType::RAWKEYDOWN)
            );
            assert!(events[0].as_ref().is_some_and(|event| event.character != 0));
            assert!(events[1].is_none());
        }
    }

    #[test]
    fn macos_modifier_events_keep_cocoa_flags_changed_semantics() {
        for key in [Key::SHIFT, Key::CTRL, Key::ALT, Key::META, Key::CAPSLOCK] {
            for is_pressed in [true, false] {
                let events = key_events(key_input(key, 0, is_pressed), KeyboardPlatform::MacOS);
                assert_eq!(
                    events[0]
                        .as_ref()
                        .map(|event| (event.character, event.unmodified_character)),
                    Some((0, 0))
                );
                assert!(events[1].is_none());
            }
        }
    }

    #[test]
    fn macos_char_events_keep_physical_identity_and_unicode() {
        // A layout can put logical A at the physical Q position.
        let mut input = key_input(Key::A, u32::from(b'a'), true);
        input.physical_keycode = Key::Q;
        let events = key_events(input, KeyboardPlatform::MacOS);
        assert_eq!(
            events[1].as_ref().map(|event| event.type_),
            Some(KeyEventType::CHAR)
        );
        for event in events.into_iter().flatten() {
            assert_eq!(event.native_key_code, 0x0C);
            assert_eq!(event.character, u16::from(b'a'));
        }
        input.is_pressed = false;
        input.unicode = 0;
        let events = key_events(input, KeyboardPlatform::MacOS);
        assert_eq!(
            events[0].as_ref().map(|event| event.native_key_code),
            Some(0x0C)
        );

        // Shifted punctuation has a different logical key but the same hardware key.
        input = key_input(Key::EXCLAM, 0, false);
        input.physical_keycode = Key::KEY_1;
        input.modifiers =
            crate::compat::event_flags_to_u32(cef_event_flags_t::EVENTFLAG_SHIFT_DOWN);
        let events = key_events(input, KeyboardPlatform::MacOS);
        assert_eq!(
            events[0]
                .as_ref()
                .map(|event| (event.native_key_code, event.character)),
            Some((0x12, u16::from(b'!')))
        );
    }

    #[test]
    fn macos_character_fallback_does_not_insert_text_or_replace_real_unicode() {
        let mut input = key_input(Key::A, 0, true);
        let events = key_events(input, KeyboardPlatform::MacOS);
        assert!(events[1].is_none());
        input.unicode = 'å' as u32;
        let events = key_events(input, KeyboardPlatform::MacOS);
        assert_eq!(
            events[1].as_ref().map(|event| event.character),
            Some('å' as u16)
        );
        input.focus_on_editable_field = true;
        let events = key_events(input, KeyboardPlatform::MacOS);
        assert!(events[1].is_none());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn navigation_keyup_is_forwarded_on_other_platforms() {
        for key in [
            Key::UP,
            Key::DOWN,
            Key::LEFT,
            Key::RIGHT,
            Key::HOME,
            Key::END,
            Key::PAGEUP,
            Key::PAGEDOWN,
        ] {
            let events = key_events(key_input(key, 0, false), KeyboardPlatform::Other);
            assert_eq!(
                events[0].as_ref().map(|event| event.type_),
                Some(KeyEventType::KEYUP)
            );
            assert!(events[1].is_none());
        }
    }

    #[test]
    fn ime_proxy_receives_text_but_not_browser_navigation_or_shortcuts() {
        assert!(should_deliver_key_to_ime_proxy(Key::A, false, false, false));
        assert!(should_deliver_key_to_ime_proxy(
            Key::NONE,
            false,
            false,
            false
        ));
        assert!(should_deliver_key_to_ime_proxy(Key::E, true, true, false));
        assert!(!should_deliver_key_to_ime_proxy(
            Key::TAB,
            false,
            false,
            false
        ));
        assert!(!should_deliver_key_to_ime_proxy(Key::V, true, false, false));
        assert!(!should_deliver_key_to_ime_proxy(Key::V, false, false, true));
        assert!(!should_deliver_key_to_ime_proxy(Key::A, true, false, false));
    }

    #[test]
    fn cancelling_touches_terminates_each_id_and_rejects_stale_events() {
        let mut ids = HashMap::new();
        assert_eq!(
            touch_id_for_event(&mut ids, 2, TouchEventType::PRESSED),
            Some(2)
        );
        assert_eq!(
            touch_id_for_event(&mut ids, 7, TouchEventType::PRESSED),
            Some(7)
        );
        assert_eq!(
            touch_id_for_event(&mut ids, 2, TouchEventType::PRESSED),
            None
        );
        let mut cancelled = cancelled_touch_events(&mut ids);
        cancelled.sort_by_key(|event| event.id);
        assert_eq!(
            cancelled.iter().map(|event| event.id).collect::<Vec<_>>(),
            vec![2, 7]
        );
        assert!(
            cancelled
                .iter()
                .all(|event| event.type_ == TouchEventType::CANCELLED)
        );
        assert!(cancelled_touch_events(&mut ids).is_empty());
        assert_eq!(touch_id_for_event(&mut ids, 2, TouchEventType::MOVED), None);
        assert_eq!(
            touch_id_for_event(&mut ids, 7, TouchEventType::RELEASED),
            None
        );
        assert_eq!(
            touch_id_for_event(&mut ids, 2, TouchEventType::PRESSED),
            Some(2)
        );
        assert_eq!(
            touch_id_for_event(&mut ids, 2, TouchEventType::RELEASED),
            Some(2)
        );
        assert!(ids.is_empty());
    }

    #[test]
    fn test_get_control_char_code() {
        assert_eq!(get_control_char_code(Key::BACKSPACE), 0x08);
        assert_eq!(get_control_char_code(Key::TAB), 0x09);
        assert_eq!(get_control_char_code(Key::ENTER), 0x0D);
        assert_eq!(get_control_char_code(Key::KP_ENTER), 0x0D);
        assert_eq!(get_control_char_code(Key::ESCAPE), 0x1B);
        assert_eq!(get_control_char_code(Key::DELETE), 0x7F);
        // Non-control keys return 0
        assert_eq!(get_control_char_code(Key::A), 0);
        assert_eq!(get_control_char_code(Key::SPACE), 0);
    }

    #[test]
    fn test_is_navigation_key() {
        assert!(is_navigation_key(Key::UP));
        assert!(is_navigation_key(Key::DOWN));
        assert!(is_navigation_key(Key::LEFT));
        assert!(is_navigation_key(Key::RIGHT));
        assert!(is_navigation_key(Key::HOME));
        assert!(is_navigation_key(Key::END));
        assert!(is_navigation_key(Key::PAGEUP));
        assert!(is_navigation_key(Key::PAGEDOWN));
        // Non-navigation keys
        assert!(!is_navigation_key(Key::A));
        assert!(!is_navigation_key(Key::ENTER));
        assert!(!is_navigation_key(Key::SPACE));
    }

    #[test]
    fn test_is_modifier_key() {
        assert!(is_modifier_key(Key::SHIFT));
        assert!(is_modifier_key(Key::CTRL));
        assert!(is_modifier_key(Key::ALT));
        assert!(is_modifier_key(Key::META));
        assert!(is_modifier_key(Key::CAPSLOCK));
        assert!(is_modifier_key(Key::NUMLOCK));
        assert!(is_modifier_key(Key::SCROLLLOCK));
        // Non-modifier keys
        assert!(!is_modifier_key(Key::A));
        assert!(!is_modifier_key(Key::ENTER));
    }

    #[test]
    fn test_is_keypad_key() {
        assert!(is_keypad_key(Key::KP_0));
        assert!(is_keypad_key(Key::KP_9));
        assert!(is_keypad_key(Key::KP_MULTIPLY));
        assert!(is_keypad_key(Key::KP_ADD));
        assert!(is_keypad_key(Key::KP_SUBTRACT));
        assert!(is_keypad_key(Key::KP_PERIOD));
        assert!(is_keypad_key(Key::KP_DIVIDE));
        assert!(is_keypad_key(Key::KP_ENTER));
        // Non-keypad keys
        assert!(!is_keypad_key(Key::KEY_0));
        assert!(!is_keypad_key(Key::ENTER));
    }

    #[test]
    fn test_should_send_char_event() {
        // Printable characters with unicode > 0 should send CHAR
        assert!(should_send_char_event(Key::A, 'a' as u32));
        assert!(should_send_char_event(Key::SPACE, ' ' as u32));
        // Modifier keys should never send CHAR
        assert!(!should_send_char_event(Key::SHIFT, 0));
        assert!(!should_send_char_event(Key::CTRL, 0));
        // Navigation keys should never send CHAR
        assert!(!should_send_char_event(Key::UP, 0));
        assert!(!should_send_char_event(Key::LEFT, 0));
        // Non-printable, non-modifier, non-navigation keys with unicode=0 should not send CHAR
        assert!(!should_send_char_event(Key::F1, 0));
    }

    /// Makes a pan gesture delta as Godot does on macOS: the factor is applied in f64,
    /// and the result is stored in an f32 `Vector2` with the opposite sign.
    fn godot_macos_pan_delta(
        scrolling_delta_x: f64,
        scrolling_delta_y: f64,
        factor: f64,
    ) -> Vector2 {
        Vector2::new(
            -(scrolling_delta_x * factor) as f32,
            -(scrolling_delta_y * factor) as f32,
        )
    }

    #[test]
    fn macos_pan_delta_factor_follows_godot_version() {
        assert_eq!(macos_pan_delta_factor(4, 6), 0.03);
        assert_eq!(macos_pan_delta_factor(4, 7), 0.03);
        assert_eq!(macos_pan_delta_factor(4, 8), 1.0);
        assert_eq!(macos_pan_delta_factor(5, 0), 1.0);
    }

    #[test]
    fn macos_pan_wheel_delta_restores_scrolling_delta() {
        // Godot 4.7 multiplies by 0.03 in f64. Godot 4.8 sends the delta unchanged.
        for (minor, godot_factor) in [(7, 0.03), (8, 1.0)] {
            let factor = macos_pan_delta_factor(4, minor);
            for points in -200..=200 {
                let delta = godot_macos_pan_delta(0.0, f64::from(points), godot_factor);
                assert_eq!(macos_pan_wheel_delta(delta, factor), (0, points));
                let delta = godot_macos_pan_delta(f64::from(points), 0.0, godot_factor);
                assert_eq!(macos_pan_wheel_delta(delta, factor), (points, 0));
            }
        }
    }

    #[test]
    fn macos_pan_wheel_delta_rounds_partial_points() {
        let factor = macos_pan_delta_factor(4, 7);
        let delta = godot_macos_pan_delta(0.4, 2.6, 0.03);
        assert_eq!(macos_pan_wheel_delta(delta, factor), (0, 3));
        let delta = godot_macos_pan_delta(-0.6, -2.4, 0.03);
        assert_eq!(macos_pan_wheel_delta(delta, factor), (-1, -2));
    }
}
