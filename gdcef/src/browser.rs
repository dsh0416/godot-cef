//! Browser state management for CEF integration.
//!
//! This module contains the core state types used by CefTexture for managing
//! the browser instance and rendering mode.

use cef_app::{CursorType, FrameBuffer};
use godot::classes::{ImageTexture, LineEdit, Texture2Drd};
use godot::prelude::*;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use winit::dpi::PhysicalSize;

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
use crate::accelerated_osr::{GodotTextureImporter, PlatformSharedTextureInfo};

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
        /// Shared texture info from CEF's accelerated paint callback.
        texture_info: Arc<Mutex<PlatformSharedTextureInfo>>,
        /// Platform-specific texture importer for GPU-to-GPU copy.
        importer: GodotTextureImporter,
        /// The RenderingDevice texture RID (for native handle access).
        rd_texture_rid: Rid,
        /// The Texture2DRD wrapper for display in TextureRect.
        texture_2d_rd: Gd<Texture2Drd>,
        /// Current texture width.
        texture_width: u32,
        /// Current texture height.
        texture_height: u32,
    },
}

/// Application state for the CEF browser instance.
///
/// Contains all the shared state needed for browser operation, including
/// the browser handle, rendering resources, and input state.
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
    /// Last known logical size for change detection.
    pub last_size: Vector2,
    /// Last known DPI for change detection.
    pub last_dpi: f32,
    /// Last known cursor type for change detection.
    pub last_cursor: CursorType,
    /// Last known max FPS for change detection.
    pub last_max_fps: i32,
    /// Whether IME is currently active (using LineEdit proxy).
    pub ime_active: bool,
    /// Hidden LineEdit used as IME input proxy.
    pub ime_proxy: Option<Gd<LineEdit>>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            browser: None,
            render_mode: None,
            render_size: None,
            device_scale_factor: None,
            cursor_type: None,
            message_queue: None,
            url_change_queue: None,
            title_change_queue: None,
            loading_state_queue: None,
            ime_enable_queue: None,
            ime_composition_range: None,
            last_size: Vector2::ZERO,
            last_dpi: 1.0,
            last_cursor: CursorType::Arrow,
            last_max_fps: 0,
            ime_active: false,
            ime_proxy: None,
        }
    }
}
