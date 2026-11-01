//! Trusted DOM keyboard transitions from the public Godot input entry points.

use super::*;
use godot::classes::InputEventKey;
use godot::global::Key;

// The final Enter release makes the page report its complete event sequence.
// Every release deliberately has unicode=0, as real Godot macOS releases do.
const KEYS: &[(Key, u32, &str)] = &[
    (Key::A, 'a' as u32, "a"),
    (Key::KEY_1, '1' as u32, "1"),
    (Key::SPACE, ' ' as u32, " "),
    (Key::UP, 0, "ArrowUp"),
    (Key::DOWN, 0, "ArrowDown"),
    (Key::LEFT, 0, "ArrowLeft"),
    (Key::RIGHT, 0, "ArrowRight"),
    (Key::HOME, 0, "Home"),
    (Key::END, 0, "End"),
    (Key::PAGEUP, 0, "PageUp"),
    (Key::PAGEDOWN, 0, "PageDown"),
    (Key::F1, 0, "F1"),
    (Key::SHIFT, 0, "Shift"),
    (Key::ENTER, 0, "Enter"),
];

#[derive(Default)]
pub(super) struct KeyboardState {
    phase: usize,
    focus_requested: bool,
    sent: usize,
}

impl Runner {
    fn keyboard_web(&self, kind: &str) -> Option<Value> {
        self.web_events
            .iter()
            .find(|event| event["type"] == kind && event["phase"] == self.keyboard.phase)
            .cloned()
    }

    pub(super) fn tick_keyboard_release(&mut self) {
        if !self
            .web_events
            .iter()
            .any(|event| event["type"] == "keyboard_ready")
        {
            return;
        }
        if !self.keyboard.focus_requested {
            self.keyboard.focus_requested = true;
            self.dialog_click(Vector2::new(
                50.0,
                if self.keyboard.phase == 0 {
                    34.0
                } else {
                    100.0
                },
            ));
            return;
        }
        if self.keyboard_web("keyboard_focus").is_none() {
            return;
        }
        if self.keyboard.sent < KEYS.len() * 2 {
            let (key, unicode, _) = KEYS[self.keyboard.sent / 2];
            let pressed = self.keyboard.sent.is_multiple_of(2);
            let mut event = InputEventKey::new_gd();
            event.set_keycode(key);
            event.set_physical_keycode(key);
            event.set_pressed(pressed);
            event.set_unicode(if pressed { unicode } else { 0 });
            event.set_shift_pressed(key == Key::SHIFT && pressed);
            if self.class == "CefTexture2D" {
                self.call(
                    "forward_key_event",
                    &[event.to_variant(), (self.keyboard.phase == 1).to_variant()],
                );
            } else if let Some(window) = &mut self.dialogs.window {
                window.push_input_ex(&event).in_local_coords(true).done();
            }
            self.keyboard.sent += 1;
            return;
        }
        let Some(result) = self.keyboard_web("keyboard_result") else {
            return;
        };
        let Some(events) = result["events"].as_array() else {
            self.fail("Keyboard fixture must return its DOM event sequence");
            return;
        };
        let phase = self.keyboard.phase;
        self.check(
            events.len() == KEYS.len() * 2,
            format!("Keyboard phase {phase} must deliver exactly one keydown and keyup per key: {events:?}"),
        );
        for (index, (_, _, key)) in KEYS.iter().enumerate() {
            let pair = events.get(index * 2..index * 2 + 2);
            self.check(
                pair.is_some_and(|pair| {
                    pair[0]["type"] == "keydown"
                        && pair[1]["type"] == "keyup"
                        && pair[0]["key"] == *key
                        && pair[1]["key"] == *key
                        && pair[0]["code"] == pair[1]["code"]
                        && pair[0]["keyCode"] == pair[1]["keyCode"]
                        && pair.iter().all(|event| {
                            event["trusted"] == true
                                && event["repeat"] == false
                                && event["target"] == if phase == 0 { "surface" } else { "editor" }
                        })
                }),
                format!("Keyboard phase {phase}, key {key:?} must have matching trusted down/up identity: {pair:?}"),
            );
        }
        if phase == 1 {
            self.stop();
        } else {
            self.keyboard = KeyboardState {
                phase: 1,
                ..Default::default()
            };
        }
    }

    pub(super) fn tick_keyboard_editing(&mut self) {
        // 0 saves clipboard string formats without exposing them to the harness;
        // 8 restores them. Defer assertions until restoration has completed.
        const ACTIONS: &[Key] = &[
            Key::V,
            Key::ENTER,
            Key::KP_ENTER,
            Key::A,
            Key::C,
            Key::V,
            Key::A,
            Key::V,
            Key::C,
        ];
        if !self
            .web_events
            .iter()
            .any(|event| event["type"] == "editing_ready")
        {
            return;
        }
        let phase = self.keyboard.phase;
        if !self.keyboard.focus_requested {
            self.keyboard.focus_requested = true;
            if phase == 0 {
                self.dialog_click(Vector2::new(50.0, 34.0));
            }
            self.eval(&format!("window.prepareKeyboardEditing({phase})"));
            return;
        }
        if self.keyboard_web("editing_focus").is_none() {
            return;
        }
        if self.keyboard.sent < 2 {
            let key = ACTIONS[phase];
            let shortcut = key != Key::ENTER && key != Key::KP_ENTER;
            let pressed = self.keyboard.sent == 0;
            let mut event = InputEventKey::new_gd();
            event.set_keycode(key);
            event.set_physical_keycode(key);
            event.set_pressed(pressed);
            // Shortcuts and Enter are valid Godot key events without Unicode.
            event.set_unicode(0);
            event.set_ctrl_pressed(shortcut && !cfg!(target_os = "macos"));
            event.set_meta_pressed(shortcut && cfg!(target_os = "macos"));
            if self.class == "CefTexture2D" {
                self.call(
                    "forward_key_event",
                    &[event.to_variant(), true.to_variant()],
                );
            } else if let Some(window) = &mut self.dialogs.window {
                window.push_input_ex(&event).in_local_coords(true).done();
            }
            self.keyboard.sent += 1;
            return;
        }
        let Some(result) = self.keyboard_web("editing_result") else {
            return;
        };
        if phase == 0 && result["clipboardRestorable"] != true {
            self.fail("Clipboard must deliver a paste event and contain only restorable string formats; no clipboard writes performed");
            return;
        }
        if phase + 1 < ACTIONS.len() {
            self.keyboard = KeyboardState {
                phase: phase + 1,
                ..Default::default()
            };
            return;
        }
        for (phase, key) in ACTIONS.iter().enumerate() {
            let Some(result) = self
                .web_events
                .iter()
                .find(|event| event["type"] == "editing_result" && event["phase"] == phase)
                .cloned()
            else {
                self.fail(format!("Editing phase {phase} did not report a result"));
                return;
            };
            let shortcut = *key != Key::ENTER && *key != Key::KP_ENTER;
            let expected_code = match *key {
                Key::ENTER => "Enter",
                Key::KP_ENTER => "NumpadEnter",
                Key::A => "KeyA",
                Key::C => "KeyC",
                Key::V => "KeyV",
                _ => "",
            };
            self.check(
                result["events"].as_array().is_some_and(|events| {
                    events.len() == 2
                        && events[0]["type"] == "keydown"
                        && events[1]["type"] == "keyup"
                        && events.iter().all(|event| {
                            event["trusted"] == true
                                && event["repeat"] == false
                                && event["code"] == expected_code
                                && event["ctrl"] == (shortcut && !cfg!(target_os = "macos"))
                                && event["meta"] == (shortcut && cfg!(target_os = "macos"))
                        })
                }),
                format!("Editing phase {phase} must deliver one trusted keydown/keyup pair with platform shortcut modifiers: {}", result["events"]),
            );
            let clipboard_events = result["clipboardEvents"].as_array();
            let expected_clipboard = match phase {
                0 | 5 => Some("paste"),
                4 | 8 => Some("copy"),
                _ => None,
            };
            self.check(
                clipboard_events.is_some_and(|events| match expected_clipboard {
                    Some(kind) => {
                        events.len() == 1
                            && events[0]["type"] == kind
                            && events[0]["trusted"] == true
                    }
                    None => events.is_empty(),
                }),
                format!(
                    "Editing phase {phase} must dispatch exactly the expected clipboard event: {}",
                    result["clipboardEvents"]
                ),
            );
            let correct_effect = match phase {
                0 => result["value"] == "",
                1 | 2 => {
                    result["value"] == "alpha\nomega" && result["start"] == 6 && result["end"] == 6
                }
                3 => result["value"] == "select me" && result["start"] == 0 && result["end"] == 9,
                4 => result["value"] == "gdcef-clipboard:✓",
                5 => result["value"] == "before:gdcef-clipboard:✓after",
                6 => {
                    result["value"] == "custom editor"
                        && result["start"] == 2
                        && result["end"] == 6
                        && result["customActions"] == 1
                }
                7 => result["value"] == "blocked paste" && result["customActions"] == 1,
                8 => result["clipboardRestorable"] == true,
                _ => false,
            };
            self.check(
                correct_effect,
                format!(
                    "Editing phase {phase} must apply exactly its browser/editor action: {result}"
                ),
            );
        }
        self.stop();
    }
}
