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
use crate::accelerated_osr::{GodotTextureImporter, PlatformSharedTextureInfo};

/// Queue for IPC messages from the browser to Godot.
pub type MessageQueue = Arc<Mutex<VecDeque<String>>>;

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
    /// Last known logical size for change detection.
    pub last_size: Vector2,
    /// Last known DPI for change detection.
    pub last_dpi: f32,
    /// Last known cursor type for change detection.
    pub last_cursor: CursorType,
    /// Last known max FPS for change detection.
    pub last_max_fps: i32,
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
            last_size: Vector2::ZERO,
            last_dpi: 1.0,
            last_cursor: CursorType::Arrow,
            last_max_fps: 0,
        }
    }
}
