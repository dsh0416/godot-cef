//! Browser state management for CEF integration.
//!
//! This module contains the core state types used by CefTexture for managing
//! the browser instance and rendering mode.

use cef_app::{CursorType, FrameBuffer, PhysicalSize, PopupState};
use godot::classes::{ImageTexture, Texture2Drd};
use godot::prelude::*;
use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
use crate::accelerated_osr::AcceleratedRenderState;

/// Represents a loading state event from the browser.
#[derive(Debug, Clone)]
pub enum LoadingStateEvent {
    /// Page started loading.
    Started { url: String },
    /// Page finished loading.
    Finished { url: String, http_status_code: i32 },
    /// Page load error.
    Error {
        url: String,
        error_code: i32,
        error_text: String,
    },
}

/// IME composition range info for caret positioning.
#[derive(Clone, Copy, Debug)]
pub struct ImeCompositionRange {
    /// Caret X position in view coordinates.
    pub caret_x: i32,
    /// Caret Y position in view coordinates.
    pub caret_y: i32,
    /// Caret height in pixels.
    pub caret_height: i32,
}

#[derive(Debug, Clone)]
pub struct ConsoleMessageEvent {
    pub level: u32,
    pub message: String,
    pub source: String,
    pub line: i32,
}

#[derive(Debug, Clone, Default)]
pub struct DragDataInfo {
    pub is_link: bool,
    pub is_file: bool,
    pub is_fragment: bool,
    pub link_url: String,
    pub link_title: String,
    pub fragment_text: String,
    pub fragment_html: String,
    pub file_names: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum DragEvent {
    Started {
        drag_data: DragDataInfo,
        x: i32,
        y: i32,
        allowed_ops: u32,
    },
    UpdateCursor {
        operation: u32,
    },
    Entered {
        drag_data: DragDataInfo,
        mask: u32,
    },
}

#[derive(Debug, Clone)]
pub struct DownloadRequestEvent {
    pub id: u32,
    pub url: String,
    pub original_url: String,
    pub suggested_file_name: String,
    pub mime_type: String,
    pub total_bytes: i64,
}

#[derive(Debug, Clone)]
pub struct DownloadUpdateEvent {
    pub id: u32,
    pub url: String,
    pub full_path: String,
    pub received_bytes: i64,
    pub total_bytes: i64,
    pub current_speed: i64,
    pub percent_complete: i32,
    pub is_in_progress: bool,
    pub is_complete: bool,
    pub is_canceled: bool,
}

/// Consolidated event queues for browser-to-Godot communication.
///
/// All UI-thread callbacks write to this single structure, which is then
/// drained once per frame in `on_process`. This reduces lock overhead
/// compared to having separate `Arc<Mutex<...>>` for each queue.
#[derive(Default)]
pub struct EventQueues {
    /// IPC messages from the browser (string).
    pub messages: VecDeque<String>,
    /// Binary IPC messages from the browser.
    pub binary_messages: VecDeque<Vec<u8>>,
    /// URL change notifications.
    pub url_changes: VecDeque<String>,
    /// Title change notifications.
    pub title_changes: VecDeque<String>,
    /// Loading state events.
    pub loading_states: VecDeque<LoadingStateEvent>,
    /// IME enable/disable requests.
    pub ime_enables: VecDeque<bool>,
    /// IME composition range (latest value wins).
    pub ime_composition_range: Option<ImeCompositionRange>,
    /// Console messages.
    pub console_messages: VecDeque<ConsoleMessageEvent>,
    /// Drag events.
    pub drag_events: VecDeque<DragEvent>,
    /// Download request events.
    pub download_requests: VecDeque<DownloadRequestEvent>,
    /// Download update events.
    pub download_updates: VecDeque<DownloadUpdateEvent>,
    /// Render process terminated event.
    pub render_process_terminated: VecDeque<(String, cef::TerminationStatus)>, // (reason, status)
}

impl EventQueues {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Shared handle to consolidated event queues.
pub type EventQueuesHandle = Arc<Mutex<EventQueues>>;

/// Audio parameters from CEF audio stream.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct AudioParameters {
    pub channels: i32,
    pub sample_rate: i32,
    pub frames_per_buffer: i32,
}

/// Audio packet containing interleaved stereo f32 PCM data from CEF.
#[derive(Clone)]
#[allow(dead_code)]
pub struct AudioPacket {
    pub data: Vec<f32>,
    pub frames: i32,
    pub pts: i64,
}

/// Queue for audio packets from the browser to Godot.
/// Kept separate because audio callbacks may run on different threads.
pub type AudioPacketQueue = Arc<Mutex<VecDeque<AudioPacket>>>;

/// Shared audio parameters from CEF.
pub type AudioParamsState = Arc<Mutex<Option<AudioParameters>>>;

/// Shared sample rate for audio capture.
pub type AudioSampleRateState = Arc<Mutex<i32>>;

/// Shutdown flag for audio handler to suppress errors during cleanup.
pub type AudioShutdownFlag = Arc<AtomicBool>;

#[derive(Debug, Clone, Default)]
pub struct DragState {
    pub is_drag_over: bool,
    pub is_dragging_from_browser: bool,
    pub allowed_ops: u32,
}

/// Rendering mode for the CEF browser.
///
/// Determines whether the browser uses software (CPU) rendering or
/// GPU-accelerated shared texture rendering.
pub enum RenderMode {
    /// Software rendering using a CPU frame buffer.
    Software {
        /// Shared frame buffer containing RGBA pixel data.
        frame_buffer: Arc<Mutex<FrameBuffer>>,
        /// Godot ImageTexture for display.
        texture: Gd<ImageTexture>,
    },
    /// GPU-accelerated rendering using platform-specific shared textures.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    Accelerated {
        /// Shared render state containing importer and pending copy tracking.
        /// This is shared with the render handler for immediate GPU copy in on_accelerated_paint.
        render_state: Arc<Mutex<AcceleratedRenderState>>,
        /// The Texture2DRD wrapper for display in TextureRect.
        texture_2d_rd: Gd<Texture2Drd>,
    },
}

/// Shared popup state for <select> dropdowns and other browser popups.
pub type PopupStateQueue = Arc<Mutex<PopupState>>;

/// CEF browser state and shared resources.
///
/// Contains the browser handle and resources shared with CEF handlers via Arc<Mutex>.
/// Local Godot state (change detection, IME widgets) lives on CefTexture directly.
#[derive(Default)]
pub struct App {
    /// The CEF browser instance.
    pub browser: Option<cef::Browser>,
    /// Current rendering mode (software or accelerated).
    pub render_mode: Option<RenderMode>,
    /// Shared render size in physical pixels.
    pub render_size: Option<Arc<Mutex<PhysicalSize<f32>>>>,
    /// Shared device scale factor for DPI awareness.
    pub device_scale_factor: Option<Arc<Mutex<f32>>>,
    /// Shared cursor type from CEF.
    pub cursor_type: Option<Arc<Mutex<CursorType>>>,
    /// Shared popup state for <select> dropdowns.
    pub popup_state: Option<PopupStateQueue>,
    /// Consolidated event queues for browser-to-Godot communication.
    pub event_queues: Option<EventQueuesHandle>,
    /// Current drag state for this browser.
    pub drag_state: DragState,
    /// Queue for audio packets from the browser.
    /// Kept separate because audio callbacks may run on different threads.
    pub audio_packet_queue: Option<AudioPacketQueue>,
    /// Shared audio parameters from CEF.
    pub audio_params: Option<AudioParamsState>,
    /// Shared sample rate configuration (from Godot's AudioServer).
    pub audio_sample_rate: Option<AudioSampleRateState>,
    /// Shutdown flag for audio handler to suppress errors during cleanup.
    pub audio_shutdown_flag: Option<AudioShutdownFlag>,
}

impl App {
    /// Clears per-instance runtime state. This is used during `CefTexture` cleanup
    /// and can be reused by tests as a deterministic reset point.
    pub fn clear_runtime_state(&mut self) {
        self.browser = None;
        self.render_mode = None;
        self.render_size = None;
        self.device_scale_factor = None;
        self.cursor_type = None;
        self.popup_state = None;
        self.event_queues = None;
        self.drag_state = Default::default();
        self.audio_packet_queue = None;
        self.audio_params = None;
        self.audio_sample_rate = None;
        self.audio_shutdown_flag = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_runtime_state_reset_is_deterministic() {
        let mut app = App::default();
        for _ in 0..1000 {
            app.drag_state.is_drag_over = true;
            app.drag_state.is_dragging_from_browser = true;
            app.drag_state.allowed_ops = u32::MAX;
            app.event_queues = Some(Arc::new(Mutex::new(EventQueues::new())));
            app.audio_packet_queue = Some(Arc::new(Mutex::new(VecDeque::new())));
            app.audio_params = Some(Arc::new(Mutex::new(None)));
            app.audio_sample_rate = Some(Arc::new(Mutex::new(48000)));
            app.audio_shutdown_flag = Some(Arc::new(AtomicBool::new(true)));

            app.clear_runtime_state();

            assert!(app.browser.is_none());
            assert!(app.render_mode.is_none());
            assert!(app.render_size.is_none());
            assert!(app.device_scale_factor.is_none());
            assert!(app.cursor_type.is_none());
            assert!(app.popup_state.is_none());
            assert!(app.event_queues.is_none());
            assert!(!app.drag_state.is_drag_over);
            assert!(!app.drag_state.is_dragging_from_browser);
            assert_eq!(app.drag_state.allowed_ops, 0);
            assert!(app.audio_packet_queue.is_none());
            assert!(app.audio_params.is_none());
            assert!(app.audio_sample_rate.is_none());
            assert!(app.audio_shutdown_flag.is_none());
        }
    }
}
