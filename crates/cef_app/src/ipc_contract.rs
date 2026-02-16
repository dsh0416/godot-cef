//! Shared IPC contract between Godot-side and renderer-side code.
//!
//! Keeping route names and payload limits in one module avoids drift across
//! crates and process boundaries.

/// Maximum IPC data payload size in bytes (8 MiB).
pub const MAX_IPC_DATA_BYTES: usize = 8 * 1024 * 1024;

pub const ROUTE_IPC_GODOT_TO_RENDERER: &str = "ipcGodotToRenderer";
pub const ROUTE_IPC_RENDERER_TO_GODOT: &str = "ipcRendererToGodot";

pub const ROUTE_IPC_BINARY_GODOT_TO_RENDERER: &str = "ipcBinaryGodotToRenderer";
pub const ROUTE_IPC_BINARY_RENDERER_TO_GODOT: &str = "ipcBinaryRendererToGodot";

pub const ROUTE_IPC_DATA_GODOT_TO_RENDERER: &str = "ipcDataGodotToRenderer";
pub const ROUTE_IPC_DATA_RENDERER_TO_GODOT: &str = "ipcDataRendererToGodot";

pub const ROUTE_TRIGGER_IME: &str = "triggerIme";
pub const ROUTE_IME_CARET_POSITION: &str = "imeCaretPosition";
