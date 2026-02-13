//! Cross-platform compatibility helpers.
//!
//! CEF's C API uses platform-specific integer widths for several types
//! (e.g. `cef_event_flags_t` is `u32` on Linux/macOS but `i32` on Windows).
//! This module centralizes the necessary casts so the rest of the codebase
//! can work with a uniform `u32` representation.

use cef::sys::cef_event_flags_t;

/// Converts a `cef_event_flags_t` bitfield to `u32`.
///
/// On Windows the inner type is `i32`; on other platforms it is already `u32`.
#[inline]
pub fn event_flags_to_u32(flags: cef_event_flags_t) -> u32 {
    #[cfg(target_os = "windows")]
    {
        flags.0 as u32
    }
    #[cfg(not(target_os = "windows"))]
    {
        flags.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_flags_none_is_zero() {
        assert_eq!(event_flags_to_u32(cef_event_flags_t::EVENTFLAG_NONE), 0);
    }

    #[test]
    fn test_event_flags_combined() {
        let combined =
            cef_event_flags_t::EVENTFLAG_SHIFT_DOWN | cef_event_flags_t::EVENTFLAG_CONTROL_DOWN;
        let result = event_flags_to_u32(combined);
        let shift = event_flags_to_u32(cef_event_flags_t::EVENTFLAG_SHIFT_DOWN);
        let ctrl = event_flags_to_u32(cef_event_flags_t::EVENTFLAG_CONTROL_DOWN);
        assert_eq!(result, shift | ctrl);
    }

    #[test]
    fn test_event_flags_individual_bits_are_nonzero() {
        assert_ne!(
            event_flags_to_u32(cef_event_flags_t::EVENTFLAG_SHIFT_DOWN),
            0
        );
        assert_ne!(
            event_flags_to_u32(cef_event_flags_t::EVENTFLAG_CONTROL_DOWN),
            0
        );
        assert_ne!(event_flags_to_u32(cef_event_flags_t::EVENTFLAG_ALT_DOWN), 0);
    }
}
