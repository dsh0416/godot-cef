//! AppKit editing commands for a windowless macOS browser.
//!
//! Native macOS browsers receive these commands through the responder chain.
//! OSR has no native browser view in that chain, so provide the commands only
//! after Chromium and the page have declined to handle the key event.

use cef::sys::cef_event_flags_t;
use cef::{KeyEvent, KeyEventType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditCommand {
    SelectAll,
    Copy,
    Cut,
    Paste,
    PasteAndMatchStyle,
    Undo,
    Redo,
}

fn edit_command(event: &KeyEvent) -> Option<EditCommand> {
    if !matches!(
        event.type_,
        KeyEventType::RAWKEYDOWN | KeyEventType::KEYDOWN
    ) {
        return None;
    }

    let command = crate::compat::event_flags_to_u32(cef_event_flags_t::EVENTFLAG_COMMAND_DOWN);
    let shift = crate::compat::event_flags_to_u32(cef_event_flags_t::EVENTFLAG_SHIFT_DOWN);
    let shortcut_modifiers = crate::compat::event_flags_to_u32(
        cef_event_flags_t::EVENTFLAG_COMMAND_DOWN
            | cef_event_flags_t::EVENTFLAG_SHIFT_DOWN
            | cef_event_flags_t::EVENTFLAG_CONTROL_DOWN
            | cef_event_flags_t::EVENTFLAG_ALT_DOWN,
    );
    // Locks, repeat and key-location flags do not change the shortcut. Control
    // and Option do: do not steal an editor's Cmd+Option/Ctrl combinations.
    let modifiers = event.modifiers & shortcut_modifiers;
    if modifiers == command {
        match event.windows_key_code {
            0x41 => Some(EditCommand::SelectAll),
            0x43 => Some(EditCommand::Copy),
            0x58 => Some(EditCommand::Cut),
            0x56 => Some(EditCommand::Paste),
            0x5A => Some(EditCommand::Undo),
            _ => None,
        }
    } else if modifiers == command | shift {
        match event.windows_key_code {
            0x56 => Some(EditCommand::PasteAndMatchStyle),
            0x5A => Some(EditCommand::Redo),
            _ => None,
        }
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use cef::{
        Browser, ImplBrowser, ImplFrame, ImplKeyboardHandler, KeyboardHandler, WrapKeyboardHandler,
        rc::Rc, wrap_keyboard_handler,
    };

    wrap_keyboard_handler! {
        pub(crate) struct MacKeyboardHandler;

        impl KeyboardHandler {
            fn on_pre_key_event(
                &self,
                _browser: Option<&mut Browser>,
                event: Option<&KeyEvent>,
                _os_event: *mut u8,
                is_keyboard_shortcut: Option<&mut i32>,
            ) -> i32 {
                if event.and_then(edit_command).is_some()
                    && let Some(is_keyboard_shortcut) = is_keyboard_shortcut
                {
                    *is_keyboard_shortcut = 1;
                }
                // Keep the trusted keydown available to JavaScript editors.
                0
            }

            fn on_key_event(
                &self,
                browser: Option<&mut Browser>,
                event: Option<&KeyEvent>,
                _os_event: *mut u8,
            ) -> i32 {
                let Some(command) = event.and_then(edit_command) else {
                    return 0;
                };
                let Some(frame) = browser.and_then(|browser| browser.focused_frame()) else {
                    return 0;
                };
                // OnKeyEvent receives only events unhandled by the renderer;
                // preventDefault() therefore suppresses this fallback too.
                match command {
                    EditCommand::SelectAll => frame.select_all(),
                    EditCommand::Copy => frame.copy(),
                    EditCommand::Cut => frame.cut(),
                    EditCommand::Paste => frame.paste(),
                    EditCommand::PasteAndMatchStyle => frame.paste_and_match_style(),
                    EditCommand::Undo => frame.undo(),
                    EditCommand::Redo => frame.redo(),
                }
                1
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) use macos::MacKeyboardHandler;

#[cfg(test)]
mod tests {
    use super::*;

    fn key_event(key: u8, modifiers: cef_event_flags_t) -> KeyEvent {
        KeyEvent {
            type_: KeyEventType::RAWKEYDOWN,
            windows_key_code: i32::from(key),
            modifiers: crate::compat::event_flags_to_u32(modifiers),
            ..Default::default()
        }
    }

    #[test]
    fn macos_editing_shortcuts_have_explicit_commands() {
        let command = cef_event_flags_t::EVENTFLAG_COMMAND_DOWN;
        for (key, expected) in [
            (b'A', EditCommand::SelectAll),
            (b'C', EditCommand::Copy),
            (b'X', EditCommand::Cut),
            (b'V', EditCommand::Paste),
            (b'Z', EditCommand::Undo),
        ] {
            assert_eq!(edit_command(&key_event(key, command)), Some(expected));
        }
        for (key, expected) in [
            (b'V', EditCommand::PasteAndMatchStyle),
            (b'Z', EditCommand::Redo),
        ] {
            assert_eq!(
                edit_command(&key_event(
                    key,
                    command | cef_event_flags_t::EVENTFLAG_SHIFT_DOWN
                )),
                Some(expected)
            );
        }
    }

    #[test]
    fn unrelated_modifier_combinations_are_left_to_the_page() {
        let command = cef_event_flags_t::EVENTFLAG_COMMAND_DOWN;
        for modifiers in [
            cef_event_flags_t::EVENTFLAG_NONE,
            cef_event_flags_t::EVENTFLAG_CONTROL_DOWN,
            command | cef_event_flags_t::EVENTFLAG_CONTROL_DOWN,
            command | cef_event_flags_t::EVENTFLAG_ALT_DOWN,
            command
                | cef_event_flags_t::EVENTFLAG_ALT_DOWN
                | cef_event_flags_t::EVENTFLAG_SHIFT_DOWN,
        ] {
            assert_eq!(edit_command(&key_event(b'V', modifiers)), None);
        }
        assert_eq!(edit_command(&key_event(b'S', command)), None);
        assert_eq!(
            edit_command(&key_event(
                b'A',
                command | cef_event_flags_t::EVENTFLAG_SHIFT_DOWN
            )),
            None
        );
    }

    #[test]
    fn commands_run_on_keydown_only_even_when_lock_or_repeat_flags_are_present() {
        let mut event = key_event(
            b'V',
            cef_event_flags_t::EVENTFLAG_COMMAND_DOWN
                | cef_event_flags_t::EVENTFLAG_CAPS_LOCK_ON
                | cef_event_flags_t::EVENTFLAG_IS_REPEAT,
        );
        assert_eq!(edit_command(&event), Some(EditCommand::Paste));
        event.type_ = KeyEventType::KEYDOWN;
        assert_eq!(edit_command(&event), Some(EditCommand::Paste));
        for type_ in [KeyEventType::KEYUP, KeyEventType::CHAR] {
            event.type_ = type_;
            assert_eq!(edit_command(&event), None);
        }
    }
}
