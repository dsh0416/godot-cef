use std::sync::{Arc, Mutex};

use crate::types::{CursorType, FrameBuffer, PhysicalSize, PopupState};

#[derive(Clone)]
pub struct OsrRenderHandler {
    pub device_scale_factor: Arc<Mutex<f32>>,
    pub size: Arc<Mutex<PhysicalSize<f32>>>,
    pub frame_buffer: Arc<Mutex<FrameBuffer>>,
    pub cursor_type: Arc<Mutex<CursorType>>,
    pub popup_state: Arc<Mutex<PopupState>>,
}

impl OsrRenderHandler {
    pub fn new(device_scale_factor: f32, size: PhysicalSize<f32>) -> Self {
        Self {
            size: Arc::new(Mutex::new(size)),
            device_scale_factor: Arc::new(Mutex::new(device_scale_factor)),
            frame_buffer: Arc::new(Mutex::new(FrameBuffer::new())),
            cursor_type: Arc::new(Mutex::new(CursorType::default())),
            popup_state: Arc::new(Mutex::new(PopupState::new())),
        }
    }

    pub fn get_frame_buffer(&self) -> Arc<Mutex<FrameBuffer>> {
        self.frame_buffer.clone()
    }

    pub fn get_size(&self) -> Arc<Mutex<PhysicalSize<f32>>> {
        self.size.clone()
    }

    pub fn get_device_scale_factor(&self) -> Arc<Mutex<f32>> {
        self.device_scale_factor.clone()
    }

    pub fn get_cursor_type(&self) -> Arc<Mutex<CursorType>> {
        self.cursor_type.clone()
    }

    pub fn get_popup_state(&self) -> Arc<Mutex<PopupState>> {
        self.popup_state.clone()
    }
}
