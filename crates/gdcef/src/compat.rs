//! Cross-platform compatibility helpers.
//!
//! CEF's C API uses platform-specific integer widths for several types
//! (e.g. `cef_event_flags_t` is `u32` on Linux/macOS but `i32` on Windows).
//! This module centralizes the necessary casts so the rest of the codebase
//! can work with a uniform `u32` representation.

use cef::sys::cef_event_flags_t;

/// Casts a CEF "raw" integer value to `u32` across platforms.
///
/// Some CEF raw values are `i32` on Windows and `u32` elsewhere.
#[macro_export]
macro_rules! cef_raw_to_u32 {
    ($value:expr) => {{
        #[cfg(target_os = "windows")]
        {
            $value as u32
        }
        #[cfg(not(target_os = "windows"))]
        {
            $value
        }
    }};
}

/// Casts a CEF "raw" integer value to `i32` across platforms.
///
/// Some CEF raw values are `u32` on non-Windows targets.
#[macro_export]
macro_rules! cef_raw_to_i32 {
    ($value:expr) => {{
        #[cfg(target_os = "windows")]
        {
            $value
        }
        #[cfg(not(target_os = "windows"))]
        {
            $value as i32
        }
    }};
}

/// Casts an `i32` input to the platform-specific raw CEF integer type.
///
/// Useful when constructing raw CEF tuple structs that differ by target.
#[macro_export]
macro_rules! cef_i32_to_raw {
    ($value:expr) => {{
        #[cfg(target_os = "windows")]
        {
            $value
        }
        #[cfg(not(target_os = "windows"))]
        {
            $value as u32
        }
    }};
}

/// Converts a `cef_event_flags_t` bitfield to `u32`.
///
/// On Windows the inner type is `i32`; on other platforms it is already `u32`.
#[inline]
pub fn event_flags_to_u32(flags: cef_event_flags_t) -> u32 {
    crate::cef_raw_to_u32!(flags.0)
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
