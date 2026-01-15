#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

use cef::{AcceleratedPaintInfo, PaintElementType};
use godot::classes::RenderingServer;
use godot::global::godot_print;
use godot::prelude::*;
use std::sync::{Arc, Mutex};

#[cfg(target_os = "linux")]
pub use linux::GodotTextureImporter;
#[cfg(target_os = "macos")]
pub use macos::GodotTextureImporter;
#[cfg(target_os = "windows")]
pub use windows::GodotTextureImporter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderBackend {
    Metal,
    Vulkan,
    D3D12,
    OpenGL,
    Unknown,
}

impl RenderBackend {
    pub fn detect() -> Self {
        let rs = RenderingServer::singleton();
        let driver_name = rs.get_current_rendering_driver_name().to_string();
        let driver_lower = driver_name.to_lowercase();

        let backend = if driver_lower.contains("metal") {
            RenderBackend::Metal
        } else if driver_lower.contains("vulkan") {
            RenderBackend::Vulkan
        } else if driver_lower.contains("d3d12") {
            RenderBackend::D3D12
        } else if driver_lower.contains("opengl") || driver_lower.contains("gl_") {
            RenderBackend::OpenGL
        } else {
            RenderBackend::Unknown
        };

        godot_print!(
            "[AcceleratedOSR] Detected render backend: {:?} (driver: {})",
            backend,
            driver_name
        );

        backend
    }

    pub fn supports_accelerated_osr(&self) -> bool {
        match self {
            #[cfg(target_os = "macos")]
            RenderBackend::Metal => true,
            #[cfg(target_os = "windows")]
            RenderBackend::D3D12 => true,
            #[cfg(target_os = "linux")]
            RenderBackend::Vulkan => true,
            _ => false,
        }
    }
}

#[derive(Default)]
pub struct PendingCopyState {
    pub copy_id: Option<u64>,
    pub frame_pending: bool,
}

pub struct AcceleratedRenderState {
    pub importer: GodotTextureImporter,
    pub dst_rd_rid: Rid,
    pub dst_width: u32,
    pub dst_height: u32,
    pub pending_copy: PendingCopyState,
    pub needs_resize: Option<(u32, u32)>,
}

impl AcceleratedRenderState {
    pub fn new(importer: GodotTextureImporter, dst_rd_rid: Rid, width: u32, height: u32) -> Self {
        Self {
            importer,
            dst_rd_rid,
            dst_width: width,
            dst_height: height,
            pending_copy: PendingCopyState::default(),
            needs_resize: None,
        }
    }
}

#[derive(Clone)]
pub struct AcceleratedRenderHandler {
    pub device_scale_factor: Arc<Mutex<f32>>,
    pub size: Arc<Mutex<winit::dpi::PhysicalSize<f32>>>,
    pub cursor_type: Arc<Mutex<cef_app::CursorType>>,
    render_state: Option<Arc<Mutex<AcceleratedRenderState>>>,
}

impl AcceleratedRenderHandler {
    pub fn new(device_scale_factor: f32, size: winit::dpi::PhysicalSize<f32>) -> Self {
        Self {
            device_scale_factor: Arc::new(Mutex::new(device_scale_factor)),
            size: Arc::new(Mutex::new(size)),
            cursor_type: Arc::new(Mutex::new(cef_app::CursorType::default())),
            render_state: None,
        }
    }

    pub fn set_render_state(&mut self, state: Arc<Mutex<AcceleratedRenderState>>) {
        self.render_state = Some(state);
    }

    pub fn on_accelerated_paint(
        &self,
        type_: PaintElementType,
        info: Option<&AcceleratedPaintInfo>,
    ) {
        let Some(info) = info else { return };
        if type_ != PaintElementType::default() {
            return;
        }

        let src_width = info.extra.coded_size.width as u32;
        let src_height = info.extra.coded_size.height as u32;

        // Perform immediate GPU copy while handle is valid
        let Some(render_state_arc) = &self.render_state else {
            return;
        };

        let Ok(mut state) = render_state_arc.lock() else {
            godot::global::godot_error!("[AcceleratedOSR] Failed to lock render state");
            return;
        };

        // Check if texture dimensions changed - defer resize to main loop
        if src_width != state.dst_width || src_height != state.dst_height {
            state.needs_resize = Some((src_width, src_height));
            state.pending_copy.frame_pending = true;
            // Can't copy to mismatched texture, skip this frame
            return;
        }

        // Check if previous copy is still in progress
        if let Some(copy_id) = state.pending_copy.copy_id {
            if !state.importer.is_copy_complete(copy_id) {
                // Previous copy still running, skip this frame to avoid backing up
                return;
            }
            state.pending_copy.copy_id = None;
        }

        // Perform immediate import and copy while handle is guaranteed valid
        let dst_rid = state.dst_rd_rid;
        match state.importer.import_and_copy(info, dst_rid) {
            Ok(copy_id) => {
                state.pending_copy.copy_id = Some(copy_id);
                state.pending_copy.frame_pending = false;
            }
            Err(e) => {
                // Don't spam the log for device removed/suspended errors
                // (these are logged once by check_device_state)
                if !e.contains("D3D12 device removed") {
                    godot::global::godot_error!(
                        "[AcceleratedOSR] Failed to import and copy texture: {}",
                        e
                    );
                }
            }
        }
    }

    pub fn get_size(&self) -> Arc<Mutex<winit::dpi::PhysicalSize<f32>>> {
        self.size.clone()
    }

    pub fn get_device_scale_factor(&self) -> Arc<Mutex<f32>> {
        self.device_scale_factor.clone()
    }

    pub fn get_cursor_type(&self) -> Arc<Mutex<cef_app::CursorType>> {
        self.cursor_type.clone()
    }
}

pub type PlatformAcceleratedRenderHandler = AcceleratedRenderHandler;

pub fn is_accelerated_osr_supported() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::is_supported()
    }
    #[cfg(target_os = "windows")]
    {
        windows::is_supported()
    }
    #[cfg(target_os = "linux")]
    {
        linux::is_supported()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        false
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub struct GodotTextureImporter;

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
impl GodotTextureImporter {
    pub fn new() -> Option<Self> {
        None
    }

    pub fn import_and_copy(
        &mut self,
        _info: &AcceleratedPaintInfo,
        _dst_rd_rid: Rid,
    ) -> Result<u64, String> {
        Err("Accelerated OSR not supported on this platform".to_string())
    }

    pub fn is_copy_complete(&self, _copy_id: u64) -> bool {
        true
    }

    pub fn wait_for_all_copies(&self) {}
}
