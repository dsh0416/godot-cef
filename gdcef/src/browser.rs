//! Browser state management for CEF integration.
//!
//! This module contains the core state types used by CefTexture for managing
//! the browser instance and rendering mode.

use cef_app::{CursorType, FrameBuffer};
use godot::classes::{ImageTexture, Texture2Drd};
use godot::prelude::*;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use winit::dpi::PhysicalSize;

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
use crate::accelerated_osr::AcceleratedRenderState;

/// Queue for IPC messages from the browser to Godot.
pub type MessageQueue = Arc<Mutex<VecDeque<String>>>;

/// Queue for URL change notifications from the browser to Godot.
pub type UrlChangeQueue = Arc<Mutex<VecDeque<String>>>;

/// Queue for title change notifications from the browser to Godot.
pub type TitleChangeQueue = Arc<Mutex<VecDeque<String>>>;

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

/// Queue for loading state events from the browser to Godot.
pub type LoadingStateQueue = Arc<Mutex<VecDeque<LoadingStateEvent>>>;

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

pub type ImeEnableQueue = Arc<Mutex<VecDeque<bool>>>;
/// Shared state for IME composition range.
pub type ImeCompositionQueue = Arc<Mutex<Option<ImeCompositionRange>>>;

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
    /// Queue for IPC messages from the browser.
    pub message_queue: Option<MessageQueue>,
    /// Queue for URL change notifications from the browser.
    pub url_change_queue: Option<UrlChangeQueue>,
    /// Queue for title change notifications from the browser.
    pub title_change_queue: Option<TitleChangeQueue>,
    /// Queue for loading state events from the browser.
    pub loading_state_queue: Option<LoadingStateQueue>,
    /// Queue for IME enable/disable requests.
    pub ime_enable_queue: Option<ImeEnableQueue>,
    /// Shared IME composition range for caret positioning.
    pub ime_composition_range: Option<ImeCompositionQueue>,
}
