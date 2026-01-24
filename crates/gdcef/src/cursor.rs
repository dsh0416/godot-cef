use cef_app::CursorType;
use godot::classes::control::CursorShape;

/// Converts CEF cursor type to Godot cursor shape
pub fn cursor_type_to_shape(cursor_type: CursorType) -> CursorShape {
    match cursor_type {
        CursorType::Arrow => CursorShape::ARROW,
        CursorType::IBeam => CursorShape::IBEAM,
        CursorType::Hand => CursorShape::POINTING_HAND,
        CursorType::Cross => CursorShape::CROSS,
        CursorType::Wait => CursorShape::WAIT,
        CursorType::Help => CursorShape::HELP,
        CursorType::Move => CursorShape::MOVE,
        CursorType::ResizeNS => CursorShape::VSIZE,
        CursorType::ResizeEW => CursorShape::HSIZE,
        CursorType::ResizeNESW => CursorShape::BDIAGSIZE,
        CursorType::ResizeNWSE => CursorShape::FDIAGSIZE,
        CursorType::NotAllowed => CursorShape::FORBIDDEN,
        CursorType::Progress => CursorShape::BUSY,
    }
}
