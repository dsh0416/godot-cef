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
#[cfg(target_os = "linux")]
pub use linux::get_godot_device_uuid;
#[cfg(target_os = "macos")]
pub use macos::GodotTextureImporter;
#[cfg(target_os = "windows")]
pub use windows::GodotTextureImporter;
#[cfg(target_os = "windows")]
pub use windows::get_godot_adapter_luid;

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
            #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
            RenderBackend::Vulkan => true,
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            RenderBackend::Vulkan => true,
            _ => false,
        }
    }
}

pub struct AcceleratedRenderState {
    pub importer: GodotTextureImporter,
    pub dst_rd_rid: Rid,
    pub dst_width: u32,
    pub dst_height: u32,
    pub needs_resize: Option<(u32, u32)>,
    pub popup_rd_rid: Option<Rid>,
    pub popup_width: u32,
    pub popup_height: u32,
    pub popup_dirty: bool,
    pub popup_has_content: bool,
    pub needs_popup_texture: Option<(u32, u32)>,
    pub has_pending_copy: bool,
}

impl AcceleratedRenderState {
    pub fn new(importer: GodotTextureImporter, dst_rd_rid: Rid, width: u32, height: u32) -> Self {
        Self {
            importer,
            dst_rd_rid,
            dst_width: width,
            dst_height: height,
            needs_resize: None,
            popup_rd_rid: None,
            popup_width: 0,
            popup_height: 0,
            popup_dirty: false,
            popup_has_content: false,
            needs_popup_texture: None,
            has_pending_copy: false,
        }
    }

    pub fn process_pending_copy(&mut self) -> Result<(), String> {
        if !self.has_pending_copy {
            return Ok(());
        }

        self.importer.process_pending_copy(self.dst_rd_rid)?;
        self.has_pending_copy = false;
        Ok(())
    }
}

#[derive(Clone)]
pub struct AcceleratedRenderHandler {
    pub device_scale_factor: Arc<Mutex<f32>>,
    pub size: Arc<Mutex<cef_app::PhysicalSize<f32>>>,
    pub cursor_type: Arc<Mutex<cef_app::CursorType>>,
    pub popup_state: Arc<Mutex<cef_app::PopupState>>,
    render_state: Option<Arc<Mutex<AcceleratedRenderState>>>,
}

impl AcceleratedRenderHandler {
    pub fn new(device_scale_factor: f32, size: cef_app::PhysicalSize<f32>) -> Self {
        Self {
            device_scale_factor: Arc::new(Mutex::new(device_scale_factor)),
            size: Arc::new(Mutex::new(size)),
            cursor_type: Arc::new(Mutex::new(cef_app::CursorType::default())),
            popup_state: Arc::new(Mutex::new(cef_app::PopupState::new())),
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
        if type_ == PaintElementType::POPUP {
            let src_width = info.extra.coded_size.width as u32;
            let src_height = info.extra.coded_size.height as u32;

            let Some(render_state_arc) = &self.render_state else {
                return;
            };
            let Ok(mut state) = render_state_arc.lock() else {
                return;
            };

            let need_new_texture = match state.popup_rd_rid {
                None => true,
                Some(_) => state.popup_width != src_width || state.popup_height != src_height,
            };

            if need_new_texture {
                state.needs_popup_texture = Some((src_width, src_height));
                return;
            }

            // For popups, use synchronous copy (they're small and infrequent)
            if let Some(popup_rid) = state.popup_rd_rid {
                let result = state
                    .importer
                    .queue_copy(info)
                    .and_then(|_| state.importer.process_pending_copy(popup_rid))
                    .and_then(|_| state.importer.wait_for_copy());

                match result {
                    Ok(_) => {
                        state.popup_dirty = true;
                        state.popup_has_content = true;
                    }
                    Err(e) => {
                        godot::global::godot_error!(
                            "[AcceleratedOSR] Failed to import popup texture: {}",
                            e
                        );
                    }
                }
            }
            return;
        }

        if type_ != PaintElementType::VIEW {
            return;
        }

        let src_width = info.extra.coded_size.width as u32;
        let src_height = info.extra.coded_size.height as u32;

        // Queue the copy operation for deferred processing
        // This returns immediately after duplicating the handle
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
            // Note: we still queue the copy below to capture this frame.
            // The frame will be processed AFTER resize in update_texture().
        }

        // Queue the copy operation (fast - just duplicates handle)
        // The actual GPU work will be done in process_pending_copy()
        // We queue even during resize to capture the frame - dst_rd_rid will be
        // passed at processing time after any resize is complete.
        match state.importer.queue_copy(info) {
            Ok(_) => {
                state.has_pending_copy = true;
            }
            Err(e) => {
                if !e.contains("D3D12 device removed") {
                    godot::global::godot_error!(
                        "[AcceleratedOSR] Failed to queue texture copy: {}",
                        e
                    );
                }
            }
        }
    }

    pub fn get_size(&self) -> Arc<Mutex<cef_app::PhysicalSize<f32>>> {
        self.size.clone()
    }

    pub fn get_device_scale_factor(&self) -> Arc<Mutex<f32>> {
        self.device_scale_factor.clone()
    }

    pub fn get_cursor_type(&self) -> Arc<Mutex<cef_app::CursorType>> {
        self.cursor_type.clone()
    }

    pub fn get_popup_state(&self) -> Arc<Mutex<cef_app::PopupState>> {
        self.popup_state.clone()
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

    pub fn queue_copy(&mut self, _info: &AcceleratedPaintInfo) -> Result<(), String> {
        Err("Accelerated OSR not supported on this platform".to_string())
    }

    pub fn process_pending_copy(&mut self, _dst_rd_rid: Rid) -> Result<(), String> {
        Err("Accelerated OSR not supported on this platform".to_string())
    }

    pub fn wait_for_copy(&mut self) -> Result<(), String> {
        Err("Accelerated OSR not supported on this platform".to_string())
    }
}
