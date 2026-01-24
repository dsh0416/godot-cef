use godot::{global::Key, obj::EngineEnum};

/// Converts Godot key codes to Windows virtual key codes
/// CEF expects Windows virtual key codes for the windows_key_code field on all platforms
pub fn godot_key_to_windows_keycode(key: Key) -> i32 {
    match key {
        // Letters A-Z (0x41-0x5A)
        Key::A => 0x41,
        Key::B => 0x42,
        Key::C => 0x43,
        Key::D => 0x44,
        Key::E => 0x45,
        Key::F => 0x46,
        Key::G => 0x47,
        Key::H => 0x48,
        Key::I => 0x49,
        Key::J => 0x4A,
        Key::K => 0x4B,
        Key::L => 0x4C,
        Key::M => 0x4D,
        Key::N => 0x4E,
        Key::O => 0x4F,
        Key::P => 0x50,
        Key::Q => 0x51,
        Key::R => 0x52,
        Key::S => 0x53,
        Key::T => 0x54,
        Key::U => 0x55,
        Key::V => 0x56,
        Key::W => 0x57,
        Key::X => 0x58,
        Key::Y => 0x59,
        Key::Z => 0x5A,

        // Numbers 0-9 (0x30-0x39)
        Key::KEY_0 => 0x30,
        Key::KEY_1 => 0x31,
        Key::KEY_2 => 0x32,
        Key::KEY_3 => 0x33,
        Key::KEY_4 => 0x34,
        Key::KEY_5 => 0x35,
        Key::KEY_6 => 0x36,
        Key::KEY_7 => 0x37,
        Key::KEY_8 => 0x38,
        Key::KEY_9 => 0x39,

        // Function keys F1-F12 (0x70-0x7B)
        Key::F1 => 0x70,
        Key::F2 => 0x71,
        Key::F3 => 0x72,
        Key::F4 => 0x73,
        Key::F5 => 0x74,
        Key::F6 => 0x75,
        Key::F7 => 0x76,
        Key::F8 => 0x77,
        Key::F9 => 0x78,
        Key::F10 => 0x79,
        Key::F11 => 0x7A,
        Key::F12 => 0x7B,

        // Keypad numbers (0x60-0x69)
        Key::KP_0 => 0x60,
        Key::KP_1 => 0x61,
        Key::KP_2 => 0x62,
        Key::KP_3 => 0x63,
        Key::KP_4 => 0x64,
        Key::KP_5 => 0x65,
        Key::KP_6 => 0x66,
        Key::KP_7 => 0x67,
        Key::KP_8 => 0x68,
        Key::KP_9 => 0x69,

        // Keypad operators
        Key::KP_MULTIPLY => 0x6A,
        Key::KP_ADD => 0x6B,
        Key::KP_SUBTRACT => 0x6D,
        Key::KP_PERIOD => 0x6E,
        Key::KP_DIVIDE => 0x6F,

        // Control keys
        Key::BACKSPACE => 0x08,
        Key::TAB => 0x09,
        Key::ENTER | Key::KP_ENTER => 0x0D,
        Key::SHIFT => 0x10,
        Key::CTRL => 0x11,
        Key::ALT => 0x12,
        Key::PAUSE => 0x13,
        Key::CAPSLOCK => 0x14,
        Key::ESCAPE => 0x1B,
        Key::SPACE => 0x20,
        Key::PAGEUP => 0x21,
        Key::PAGEDOWN => 0x22,
        Key::END => 0x23,
        Key::HOME => 0x24,
        Key::LEFT => 0x25,
        Key::UP => 0x26,
        Key::RIGHT => 0x27,
        Key::DOWN => 0x28,
        Key::PRINT => 0x2C,
        Key::INSERT => 0x2D,
        Key::DELETE => 0x2E,
        Key::META => 0x5B, // Left Windows key

        // Punctuation and symbols
        Key::SEMICOLON => 0xBA,
        Key::EQUAL => 0xBB,
        Key::COMMA => 0xBC,
        Key::MINUS => 0xBD,
        Key::PERIOD => 0xBE,
        Key::SLASH => 0xBF,
        Key::QUOTELEFT => 0xC0, // Backtick/grave
        Key::BRACKETLEFT => 0xDB,
        Key::BACKSLASH => 0xDC,
        Key::BRACKETRIGHT => 0xDD,
        Key::APOSTROPHE => 0xDE,

        // Lock keys
        Key::NUMLOCK => 0x90,
        Key::SCROLLLOCK => 0x91,

        // Default: use the key's ordinal value
        _ => key.ord(),
    }
}

/// Converts Godot key codes to platform-specific native key codes
/// - On Windows: same as Windows virtual key codes
/// - On macOS: macOS virtual key codes (from HIToolbox/Events.h)
/// - On Linux: X11 key codes
#[cfg(target_os = "windows")]
pub fn godot_key_to_native_keycode(key: Key) -> i32 {
    godot_key_to_windows_keycode(key)
}

/// Converts Godot key codes to macOS native key codes (HIToolbox virtual key codes)
#[cfg(target_os = "macos")]
pub fn godot_key_to_native_keycode(key: Key) -> i32 {
    // macOS virtual key codes from HIToolbox/Events.h
    // These are hardware key codes, not character codes
    match key {
        // Letters (QWERTY layout positions)
        Key::A => 0x00, // kVK_ANSI_A
        Key::S => 0x01, // kVK_ANSI_S
        Key::D => 0x02, // kVK_ANSI_D
        Key::F => 0x03, // kVK_ANSI_F
        Key::H => 0x04, // kVK_ANSI_H
        Key::G => 0x05, // kVK_ANSI_G
        Key::Z => 0x06, // kVK_ANSI_Z
        Key::X => 0x07, // kVK_ANSI_X
        Key::C => 0x08, // kVK_ANSI_C
        Key::V => 0x09, // kVK_ANSI_V
        Key::B => 0x0B, // kVK_ANSI_B
        Key::Q => 0x0C, // kVK_ANSI_Q
        Key::W => 0x0D, // kVK_ANSI_W
        Key::E => 0x0E, // kVK_ANSI_E
        Key::R => 0x0F, // kVK_ANSI_R
        Key::Y => 0x10, // kVK_ANSI_Y
        Key::T => 0x11, // kVK_ANSI_T
        Key::O => 0x1F, // kVK_ANSI_O
        Key::U => 0x20, // kVK_ANSI_U
        Key::I => 0x22, // kVK_ANSI_I
        Key::P => 0x23, // kVK_ANSI_P
        Key::L => 0x25, // kVK_ANSI_L
        Key::J => 0x26, // kVK_ANSI_J
        Key::K => 0x28, // kVK_ANSI_K
        Key::N => 0x2D, // kVK_ANSI_N
        Key::M => 0x2E, // kVK_ANSI_M

        // Numbers
        Key::KEY_1 => 0x12, // kVK_ANSI_1
        Key::KEY_2 => 0x13, // kVK_ANSI_2
        Key::KEY_3 => 0x14, // kVK_ANSI_3
        Key::KEY_4 => 0x15, // kVK_ANSI_4
        Key::KEY_5 => 0x17, // kVK_ANSI_5
        Key::KEY_6 => 0x16, // kVK_ANSI_6
        Key::KEY_7 => 0x1A, // kVK_ANSI_7
        Key::KEY_8 => 0x1C, // kVK_ANSI_8
        Key::KEY_9 => 0x19, // kVK_ANSI_9
        Key::KEY_0 => 0x1D, // kVK_ANSI_0

        // Punctuation and symbols
        Key::MINUS => 0x1B,        // kVK_ANSI_Minus
        Key::EQUAL => 0x18,        // kVK_ANSI_Equal
        Key::BRACKETLEFT => 0x21,  // kVK_ANSI_LeftBracket
        Key::BRACKETRIGHT => 0x1E, // kVK_ANSI_RightBracket
        Key::SEMICOLON => 0x29,    // kVK_ANSI_Semicolon
        Key::APOSTROPHE => 0x27,   // kVK_ANSI_Quote
        Key::BACKSLASH => 0x2A,    // kVK_ANSI_Backslash
        Key::COMMA => 0x2B,        // kVK_ANSI_Comma
        Key::PERIOD => 0x2F,       // kVK_ANSI_Period
        Key::SLASH => 0x2C,        // kVK_ANSI_Slash
        Key::QUOTELEFT => 0x32,    // kVK_ANSI_Grave

        // Function keys
        Key::F1 => 0x7A,  // kVK_F1
        Key::F2 => 0x78,  // kVK_F2
        Key::F3 => 0x63,  // kVK_F3
        Key::F4 => 0x76,  // kVK_F4
        Key::F5 => 0x60,  // kVK_F5
        Key::F6 => 0x61,  // kVK_F6
        Key::F7 => 0x62,  // kVK_F7
        Key::F8 => 0x64,  // kVK_F8
        Key::F9 => 0x65,  // kVK_F9
        Key::F10 => 0x6D, // kVK_F10
        Key::F11 => 0x67, // kVK_F11
        Key::F12 => 0x6F, // kVK_F12

        // Control keys
        Key::ENTER => 0x24,     // kVK_Return
        Key::TAB => 0x30,       // kVK_Tab
        Key::SPACE => 0x31,     // kVK_Space
        Key::BACKSPACE => 0x33, // kVK_Delete (backspace on Mac)
        Key::ESCAPE => 0x35,    // kVK_Escape
        Key::META => 0x37,      // kVK_Command
        Key::SHIFT => 0x38,     // kVK_Shift
        Key::CAPSLOCK => 0x39,  // kVK_CapsLock
        Key::ALT => 0x3A,       // kVK_Option
        Key::CTRL => 0x3B,      // kVK_Control

        // Arrow keys
        Key::LEFT => 0x7B,  // kVK_LeftArrow
        Key::RIGHT => 0x7C, // kVK_RightArrow
        Key::DOWN => 0x7D,  // kVK_DownArrow
        Key::UP => 0x7E,    // kVK_UpArrow

        // Navigation keys
        Key::HOME => 0x73,     // kVK_Home
        Key::END => 0x77,      // kVK_End
        Key::PAGEUP => 0x74,   // kVK_PageUp
        Key::PAGEDOWN => 0x79, // kVK_PageDown
        Key::DELETE => 0x75,   // kVK_ForwardDelete

        // Keypad
        Key::KP_0 => 0x52,        // kVK_ANSI_Keypad0
        Key::KP_1 => 0x53,        // kVK_ANSI_Keypad1
        Key::KP_2 => 0x54,        // kVK_ANSI_Keypad2
        Key::KP_3 => 0x55,        // kVK_ANSI_Keypad3
        Key::KP_4 => 0x56,        // kVK_ANSI_Keypad4
        Key::KP_5 => 0x57,        // kVK_ANSI_Keypad5
        Key::KP_6 => 0x58,        // kVK_ANSI_Keypad6
        Key::KP_7 => 0x59,        // kVK_ANSI_Keypad7
        Key::KP_8 => 0x5B,        // kVK_ANSI_Keypad8
        Key::KP_9 => 0x5C,        // kVK_ANSI_Keypad9
        Key::KP_PERIOD => 0x41,   // kVK_ANSI_KeypadDecimal
        Key::KP_MULTIPLY => 0x43, // kVK_ANSI_KeypadMultiply
        Key::KP_ADD => 0x45,      // kVK_ANSI_KeypadPlus
        Key::KP_SUBTRACT => 0x4E, // kVK_ANSI_KeypadMinus
        Key::KP_DIVIDE => 0x4B,   // kVK_ANSI_KeypadDivide
        Key::KP_ENTER => 0x4C,    // kVK_ANSI_KeypadEnter

        // Default: return 0 for unknown keys
        _ => 0,
    }
}

/// Converts Godot key codes to Linux native key codes (X11 keycodes)
/// These are based on evdev scancodes + 8 (X11 convention)
#[cfg(target_os = "linux")]
pub fn godot_key_to_native_keycode(key: Key) -> i32 {
    // X11 keycodes are evdev scancodes + 8
    match key {
        // Letters (QWERTY layout)
        Key::A => 38,
        Key::B => 56,
        Key::C => 54,
        Key::D => 40,
        Key::E => 26,
        Key::F => 41,
        Key::G => 42,
        Key::H => 43,
        Key::I => 31,
        Key::J => 44,
        Key::K => 45,
        Key::L => 46,
        Key::M => 58,
        Key::N => 57,
        Key::O => 32,
        Key::P => 33,
        Key::Q => 24,
        Key::R => 27,
        Key::S => 39,
        Key::T => 28,
        Key::U => 30,
        Key::V => 55,
        Key::W => 25,
        Key::X => 53,
        Key::Y => 29,
        Key::Z => 52,

        // Numbers
        Key::KEY_0 => 19,
        Key::KEY_1 => 10,
        Key::KEY_2 => 11,
        Key::KEY_3 => 12,
        Key::KEY_4 => 13,
        Key::KEY_5 => 14,
        Key::KEY_6 => 15,
        Key::KEY_7 => 16,
        Key::KEY_8 => 17,
        Key::KEY_9 => 18,

        // Function keys
        Key::F1 => 67,
        Key::F2 => 68,
        Key::F3 => 69,
        Key::F4 => 70,
        Key::F5 => 71,
        Key::F6 => 72,
        Key::F7 => 73,
        Key::F8 => 74,
        Key::F9 => 75,
        Key::F10 => 76,
        Key::F11 => 95,
        Key::F12 => 96,

        // Control keys
        Key::ESCAPE => 9,
        Key::BACKSPACE => 22,
        Key::TAB => 23,
        Key::ENTER => 36,
        Key::SPACE => 65,
        Key::SHIFT => 50, // Left Shift
        Key::CTRL => 37,  // Left Ctrl
        Key::ALT => 64,   // Left Alt
        Key::META => 133, // Left Super/Windows
        Key::CAPSLOCK => 66,
        Key::NUMLOCK => 77,
        Key::SCROLLLOCK => 78,

        // Arrow keys
        Key::LEFT => 113,
        Key::UP => 111,
        Key::RIGHT => 114,
        Key::DOWN => 116,

        // Navigation keys
        Key::HOME => 110,
        Key::END => 115,
        Key::PAGEUP => 112,
        Key::PAGEDOWN => 117,
        Key::INSERT => 118,
        Key::DELETE => 119,

        // Punctuation and symbols
        Key::MINUS => 20,
        Key::EQUAL => 21,
        Key::BRACKETLEFT => 34,
        Key::BRACKETRIGHT => 35,
        Key::SEMICOLON => 47,
        Key::APOSTROPHE => 48,
        Key::BACKSLASH => 51,
        Key::COMMA => 59,
        Key::PERIOD => 60,
        Key::SLASH => 61,
        Key::QUOTELEFT => 49,

        // Keypad
        Key::KP_0 => 90,
        Key::KP_1 => 87,
        Key::KP_2 => 88,
        Key::KP_3 => 89,
        Key::KP_4 => 83,
        Key::KP_5 => 84,
        Key::KP_6 => 85,
        Key::KP_7 => 79,
        Key::KP_8 => 80,
        Key::KP_9 => 81,
        Key::KP_PERIOD => 91,
        Key::KP_MULTIPLY => 63,
        Key::KP_ADD => 86,
        Key::KP_SUBTRACT => 82,
        Key::KP_DIVIDE => 106,
        Key::KP_ENTER => 104,

        // Default: return 0 for unknown keys
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Windows virtual key code constants for testing.
    mod vk {
        pub const VK_A: i32 = 0x41;
        pub const VK_Z: i32 = 0x5A;
        pub const VK_0: i32 = 0x30;
        pub const VK_9: i32 = 0x39;
        pub const VK_RETURN: i32 = 0x0D;
        pub const VK_ESCAPE: i32 = 0x1B;
        pub const VK_SPACE: i32 = 0x20;
        pub const VK_TAB: i32 = 0x09;
        pub const VK_BACK: i32 = 0x08;
        pub const VK_F1: i32 = 0x70;
        pub const VK_F12: i32 = 0x7B;
        pub const VK_NUMPAD0: i32 = 0x60;
        pub const VK_NUMPAD9: i32 = 0x69;
    }

    #[test]
    fn test_letter_keys_windows() {
        assert_eq!(godot_key_to_windows_keycode(Key::A), vk::VK_A);
        assert_eq!(godot_key_to_windows_keycode(Key::Z), vk::VK_Z);
    }

    #[test]
    fn test_number_keys_windows() {
        assert_eq!(godot_key_to_windows_keycode(Key::KEY_0), vk::VK_0);
        assert_eq!(godot_key_to_windows_keycode(Key::KEY_9), vk::VK_9);
    }

    #[test]
    fn test_control_keys_windows() {
        assert_eq!(godot_key_to_windows_keycode(Key::ENTER), vk::VK_RETURN);
        assert_eq!(godot_key_to_windows_keycode(Key::ESCAPE), vk::VK_ESCAPE);
        assert_eq!(godot_key_to_windows_keycode(Key::SPACE), vk::VK_SPACE);
        assert_eq!(godot_key_to_windows_keycode(Key::TAB), vk::VK_TAB);
        assert_eq!(godot_key_to_windows_keycode(Key::BACKSPACE), vk::VK_BACK);
    }

    #[test]
    fn test_function_keys_windows() {
        assert_eq!(godot_key_to_windows_keycode(Key::F1), vk::VK_F1);
        assert_eq!(godot_key_to_windows_keycode(Key::F12), vk::VK_F12);
    }

    #[test]
    fn test_keypad_keys_windows() {
        assert_eq!(godot_key_to_windows_keycode(Key::KP_0), vk::VK_NUMPAD0);
        assert_eq!(godot_key_to_windows_keycode(Key::KP_9), vk::VK_NUMPAD9);
    }

    #[cfg(target_os = "macos")]
    mod macos_tests {
        use super::super::*;

        /// macOS virtual key codes from HIToolbox/Events.h.
        mod kv {
            pub const KVK_ANSI_A: i32 = 0x00;
            pub const KVK_RETURN: i32 = 0x24;
            pub const KVK_ESCAPE: i32 = 0x35;
            pub const KVK_SPACE: i32 = 0x31;
        }

        #[test]
        fn test_macos_native_keys() {
            assert_eq!(godot_key_to_native_keycode(Key::A), kv::KVK_ANSI_A);
            assert_eq!(godot_key_to_native_keycode(Key::ENTER), kv::KVK_RETURN);
            assert_eq!(godot_key_to_native_keycode(Key::ESCAPE), kv::KVK_ESCAPE);
            assert_eq!(godot_key_to_native_keycode(Key::SPACE), kv::KVK_SPACE);
        }
    }

    #[cfg(target_os = "linux")]
    mod linux_tests {
        use super::super::*;

        /// X11 keycodes (evdev + 8).
        mod xk {
            pub const XK_A: i32 = 38;
            pub const XK_RETURN: i32 = 36;
            pub const XK_ESCAPE: i32 = 9;
            pub const XK_SPACE: i32 = 65;
        }

        #[test]
        fn test_linux_native_keys() {
            assert_eq!(godot_key_to_native_keycode(Key::A), xk::XK_A);
            assert_eq!(godot_key_to_native_keycode(Key::ENTER), xk::XK_RETURN);
            assert_eq!(godot_key_to_native_keycode(Key::ESCAPE), xk::XK_ESCAPE);
            assert_eq!(godot_key_to_native_keycode(Key::SPACE), xk::XK_SPACE);
        }
    }
}
