//! Browser state management for CEF integration.
//!
//! This module contains the core state types used by CefTexture for managing
//! the browser instance and rendering mode.

use cef::ImplBrowser;
use cef_app::{CursorType, FrameBuffer, PhysicalSize, PopupState};
use godot::classes::{ImageTexture, Texture2D, Texture2Drd};
use godot::prelude::*;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64};
use std::sync::{Arc, Mutex};

use crate::cookie::CookieEvent;

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
use crate::accelerated_osr::AcceleratedRenderState;

/// Popup policy constants. These control what happens when a page tries to open a popup.
///
/// - `BLOCK` (0): Suppress all popups silently (default, backward-compatible).
/// - `REDIRECT` (1): Navigate the current browser to the popup URL instead of opening a new window.
/// - `SIGNAL_ONLY` (2): Emit the `popup_requested` signal and let GDScript decide.
pub mod popup_policy {
    pub const BLOCK: i32 = 0;
    pub const REDIRECT: i32 = 1;
    pub const SIGNAL_ONLY: i32 = 2;
}

/// Shared popup policy state, readable from the CEF IO thread.
pub type PopupPolicyFlag = Arc<AtomicI32>;

/// Default permission policy constants for handling browser permission prompts.
pub mod permission_policy {
    pub const DENY_ALL: i32 = 0;
    pub const ALLOW_ALL: i32 = 1;
    pub const SIGNAL: i32 = 2;
}

/// Shared default permission policy state, readable from the CEF UI thread.
pub type PermissionPolicyFlag = Arc<AtomicI32>;

/// Monotonic request-id counter for permission requests.
pub type PermissionRequestIdCounter = Arc<AtomicI64>;

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

/// Represents a popup window request from the browser.
///
/// Emitted when a page calls `window.open()` or a link has `target="_blank"`.
/// CEF's `WindowOpenDisposition` is mapped to an i32 for GDScript compatibility.
#[derive(Debug, Clone)]
pub struct PopupRequestEvent {
    /// The URL the popup wants to navigate to.
    pub target_url: String,
    /// How the browser requested the window to be opened
    /// (e.g., new foreground tab, new background tab, new popup, new window).
    /// Matches CEF's `cef_window_open_disposition_t` values.
    pub disposition: cef::WindowOpenDisposition,
    /// Whether the popup was triggered by a user gesture (click, etc.).
    pub user_gesture: bool,
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

#[derive(Debug, Clone)]
pub struct PermissionRequestEvent {
    pub permission_type: String,
    pub url: String,
    pub request_id: i64,
}

#[derive(Debug, Clone)]
pub struct FindResultEvent {
    pub count: i32,
    pub active_index: i32,
    pub final_update: bool,
}

#[derive(Clone)]
pub enum PendingPermissionDecision {
    Media {
        callback: cef::MediaAccessCallback,
        permission_bit: u32,
        callback_token: usize,
    },
    Prompt {
        callback: cef::PermissionPromptCallback,
        prompt_id: u64,
        callback_token: usize,
    },
}

pub type PendingPermissionRequests = Arc<Mutex<HashMap<i64, PendingPermissionDecision>>>;

#[derive(Clone)]
pub enum PendingPermissionAggregate {
    Media {
        callback: cef::MediaAccessCallback,
        granted_mask: u32,
    },
    Prompt {
        callback: cef::PermissionPromptCallback,
        all_granted: bool,
    },
}

impl PendingPermissionAggregate {
    pub fn new_media(callback: cef::MediaAccessCallback, granted_mask: u32) -> Self {
        Self::Media {
            callback,
            granted_mask,
        }
    }

    pub fn new_prompt(callback: cef::PermissionPromptCallback, all_granted: bool) -> Self {
        Self::Prompt {
            callback,
            all_granted,
        }
    }
}

/// Per-callback aggregation state used to resolve multi-permission requests.
pub type PendingPermissionAggregates = Arc<Mutex<HashMap<usize, PendingPermissionAggregate>>>;

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
    /// Typed IPC data messages from the browser encoded as CBOR bytes.
    pub data_messages: VecDeque<Vec<u8>>,
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
    /// Popup window request events.
    pub popup_requests: VecDeque<PopupRequestEvent>,
    /// Download request events.
    pub download_requests: VecDeque<DownloadRequestEvent>,
    /// Download update events.
    pub download_updates: VecDeque<DownloadUpdateEvent>,
    /// Permission request events.
    pub permission_requests: VecDeque<PermissionRequestEvent>,
    /// Find-in-page result events.
    pub find_results: VecDeque<FindResultEvent>,
    /// Cookie operation results.
    pub cookie_events: VecDeque<CookieEvent>,
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
pub type AudioSampleRateState = Arc<Mutex<f32>>;

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

impl RenderMode {
    pub fn texture_2d(&self) -> Gd<Texture2D> {
        match self {
            Self::Software { texture, .. } => texture.clone().upcast(),
            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            Self::Accelerated { texture_2d_rd, .. } => texture_2d_rd.clone().upcast(),
        }
    }
}

/// Shared popup state for <select> dropdowns and other browser popups.
pub type PopupStateQueue = Arc<Mutex<PopupState>>;

/// Audio capture state for a browser instance.
///
/// Groups the audio-related shared resources that are always created and
/// destroyed together when audio capture is enabled.
pub struct AudioState {
    /// Queue for audio packets from the browser.
    pub packet_queue: AudioPacketQueue,
    /// Shared sample rate configuration (from Godot's AudioServer).
    pub sample_rate: AudioSampleRateState,
    /// Shutdown flag for audio handler to suppress errors during cleanup.
    pub shutdown_flag: AudioShutdownFlag,
}

/// Active browser state containing the browser handle and shared resources.
///
/// All fields are non-optional because they are always created together
/// when a browser is initialized and destroyed together on cleanup.
pub struct BrowserState {
    /// The CEF browser instance.
    pub browser: cef::Browser,
    /// Current rendering mode (software or accelerated).
    pub render_mode: RenderMode,
    /// Shared render size in physical pixels.
    pub render_size: Arc<Mutex<PhysicalSize<f32>>>,
    /// Shared device scale factor for DPI awareness.
    pub device_scale_factor: Arc<Mutex<f32>>,
    /// Shared cursor type from CEF.
    pub cursor_type: Arc<Mutex<CursorType>>,
    /// Shared popup state for <select> dropdowns.
    pub popup_state: PopupStateQueue,
    /// Consolidated event queues for browser-to-Godot communication.
    pub event_queues: EventQueuesHandle,
    /// Audio capture state (present when audio capture is enabled).
    pub audio: Option<AudioState>,
    /// Shared popup policy flag, readable from CEF's IO thread.
    pub popup_policy: PopupPolicyFlag,
    /// Shared map of pending permission callbacks keyed by request id.
    pub pending_permission_requests: PendingPermissionRequests,
    /// Shared per-callback aggregation state for multi-permission requests.
    pub pending_permission_aggregates: PendingPermissionAggregates,
}

/// CEF browser state and shared resources.
///
/// Contains an optional browser state (present when browser is active)
/// and drag state that persists across browser lifecycle.
/// Local Godot state (change detection, IME widgets) lives on CefTexture directly.
#[derive(Default)]
pub struct App {
    /// Active browser state, present when a browser instance is running.
    pub state: Option<BrowserState>,
    /// Current drag state for this browser.
    pub drag_state: DragState,
    /// Tracks whether this instance currently holds one `cef_retain()` reference.
    pub cef_retained: bool,
}

impl App {
    /// Returns a reference to the active browser, if any.
    pub fn browser(&self) -> Option<&cef::Browser> {
        self.state.as_ref().map(|s| &s.browser)
    }

    /// Returns a mutable reference to the active browser, if any.
    pub fn browser_mut(&mut self) -> Option<&mut cef::Browser> {
        self.state.as_mut().map(|s| &mut s.browser)
    }

    /// Returns the browser host, if a browser is active.
    pub fn host(&self) -> Option<cef::BrowserHost> {
        self.state.as_ref().and_then(|s| s.browser.host())
    }

    /// Clears per-instance runtime state. This is used during `CefTexture` cleanup
    /// and can be reused by tests as a deterministic reset point.
    pub fn clear_runtime_state(&mut self) {
        self.state = None;
        self.drag_state = Default::default();
    }

    /// Marks that this instance has successfully called `cef_retain()`.
    pub fn mark_cef_retained(&mut self) {
        self.cef_retained = true;
    }

    /// Releases CEF only when this instance currently owns a retain reference.
    pub fn release_cef_if_retained(&mut self) {
        if self.cef_retained {
            crate::cef_init::cef_release();
            self.cef_retained = false;
        }
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

            app.clear_runtime_state();

            assert!(app.state.is_none());
            assert!(!app.drag_state.is_drag_over);
            assert!(!app.drag_state.is_dragging_from_browser);
            assert_eq!(app.drag_state.allowed_ops, 0);
        }
    }
}
