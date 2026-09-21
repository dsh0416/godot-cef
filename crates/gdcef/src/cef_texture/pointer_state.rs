//! Mirrors Godot's GUI pointer capture only to report CEF leave/capture loss.

#[derive(Default)]
pub(super) struct PointerState {
    buttons: u8,
    hovered: bool,
    mouse_present: bool,
}

impl PointerState {
    pub fn has_mouse(&self) -> bool {
        self.mouse_present
    }

    pub fn enter(&mut self) {
        self.hovered = true;
    }

    pub fn motion(&mut self) {
        self.mouse_present = true;
    }

    pub fn exit(&mut self) -> bool {
        self.hovered = false;
        self.take_leave()
    }

    pub fn button(&mut self, mask: u8, pressed: bool) -> bool {
        self.mouse_present = true;
        if pressed {
            self.buttons |= mask;
        } else {
            self.buttons &= !mask;
        }
        self.take_leave()
    }

    fn take_leave(&mut self) -> bool {
        if !self.hovered && self.buttons == 0 {
            std::mem::take(&mut self.mouse_present)
        } else {
            false
        }
    }

    pub fn reset(&mut self) -> bool {
        let captured = self.buttons != 0;
        let hovered = self.hovered;
        *self = Self::default();
        // Window focus loss/pausing does not necessarily cause Godot to emit a
        // new mouse-enter on resume. Its hover notifications remain authoritative.
        self.hovered = hovered;
        captured
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leave_waits_for_release_of_all_captured_buttons() {
        let mut pointer = PointerState::default();
        pointer.enter();
        assert!(!pointer.button(1, true));
        assert!(!pointer.button(2, true));
        assert!(!pointer.exit());
        assert!(!pointer.button(1, false));
        assert!(pointer.button(2, false));
        assert!(!pointer.exit());
    }

    #[test]
    fn ordinary_hover_leaves_exactly_once() {
        let mut pointer = PointerState::default();
        pointer.enter();
        pointer.motion();
        assert!(pointer.exit());
        assert!(!pointer.exit());
    }

    #[test]
    fn focus_loss_discards_capture_before_next_interaction() {
        let mut pointer = PointerState::default();
        pointer.enter();
        pointer.button(1, true);
        assert!(pointer.reset());
        assert!(!pointer.reset());
        pointer.enter();
        pointer.motion();
        assert!(pointer.exit());
    }
}
