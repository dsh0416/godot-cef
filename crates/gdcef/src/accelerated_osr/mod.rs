#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "windows", target_os = "linux"))]
mod vulkan_common;
#[cfg(target_os = "windows")]
mod windows;

mod handoff;
mod publication;
mod snapshot_pool;

/// An exclusively leased, initialized snapshot slot. Backend capture must
/// finish all reads of CEF storage and restore the agreed source state before
/// returning. This handle is resolved on the render thread, never during paint.
#[derive(Clone, Copy)]
pub struct NativeCaptureTarget {
    pub native_handle: u64,
    pub width: u32,
    pub height: u32,
    pub format: SnapshotFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotFormat {
    Bgra8,
    Rgba8,
}

impl SnapshotFormat {
    fn from_cef(info: &AcceleratedPaintInfo) -> Result<Self, String> {
        match *info.format.as_ref() {
            cef::sys::cef_color_type_t::CEF_COLOR_TYPE_BGRA_8888 => Ok(Self::Bgra8),
            cef::sys::cef_color_type_t::CEF_COLOR_TYPE_RGBA_8888 => Ok(Self::Rgba8),
            _ => Err("Unsupported CEF accelerated pixel format".into()),
        }
    }

    pub fn rd_format(self) -> godot::classes::rendering_device::DataFormat {
        use godot::classes::rendering_device::DataFormat;
        match self {
            Self::Bgra8 => DataFormat::B8G8R8A8_SRGB,
            Self::Rgba8 => DataFormat::R8G8B8A8_SRGB,
        }
    }
}

use cef::{AcceleratedPaintInfo, PaintElementType};
use godot::classes::RenderingServer;
use godot::global::godot_print;
use godot::prelude::*;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

pub(crate) const REPAINT_VIEW: u8 = 1;
pub(crate) const REPAINT_POPUP: u8 = 2;

/// Claim one retry while holding render state. A concurrently dropped callback
/// can set the bit again; completing this attempt must not clear that newer bit.
struct RepaintAttempt {
    pending: Arc<AtomicU8>,
    bit: u8,
    captured: bool,
}

impl RepaintAttempt {
    fn claim(pending: &Arc<AtomicU8>, bit: u8) -> Self {
        pending.fetch_and(!bit, Ordering::AcqRel);
        Self {
            pending: Arc::clone(pending),
            bit,
            captured: false,
        }
    }
}

impl Drop for RepaintAttempt {
    fn drop(&mut self) {
        if !self.captured {
            self.pending.fetch_or(self.bit, Ordering::Release);
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux::GodotTextureImporter;
#[cfg(target_os = "linux")]
use linux::get_godot_gpu_device_ids as native_gpu_device_ids;
#[cfg(target_os = "macos")]
pub use macos::GodotTextureImporter;
#[cfg(target_os = "macos")]
use macos::get_godot_gpu_device_ids as native_gpu_device_ids;
#[cfg(target_os = "windows")]
pub use windows::GodotTextureImporter;
#[cfg(target_os = "windows")]
use windows::get_godot_gpu_device_ids as native_gpu_device_ids;

#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
pub fn get_godot_gpu_device_ids() -> Option<(u32, u32)> {
    crate::render::on_render_thread_sync(native_gpu_device_ids)
        .ok()
        .flatten()
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
pub struct GodotTextureImporter;

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
impl GodotTextureImporter {
    pub fn new() -> Result<Self, String> {
        Err("Accelerated OSR is unsupported on this platform".into())
    }

    pub fn capture(
        &mut self,
        _info: &AcceleratedPaintInfo,
        _target: NativeCaptureTarget,
    ) -> Result<(), String> {
        Err("Accelerated OSR is unsupported on this platform".into())
    }

    pub fn prepare_publication(&self) -> Result<(), String> {
        Err("Accelerated OSR is unsupported on this platform".into())
    }
}

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

    pub fn accelerated_osr_support_diagnostic(&self) -> (bool, String) {
        match self {
            RenderBackend::Metal if cfg!(target_os = "macos") => (
                true,
                "Metal backend supports accelerated OSR on macOS".to_string(),
            ),
            RenderBackend::D3D12 if cfg!(target_os = "windows") => (
                true,
                "D3D12 backend supports accelerated OSR on Windows".to_string(),
            ),
            RenderBackend::Vulkan if cfg!(all(target_os = "windows", target_arch = "x86_64")) => (
                true,
                "Vulkan backend supports accelerated OSR on x86_64 Windows".to_string(),
            ),
            RenderBackend::Vulkan if cfg!(all(target_os = "linux", target_arch = "x86_64")) => (
                true,
                "Vulkan backend supports accelerated OSR on x86_64 Linux".to_string(),
            ),
            RenderBackend::OpenGL => (
                false,
                "OpenGL backend is not supported for accelerated OSR".to_string(),
            ),
            RenderBackend::Unknown => (
                false,
                "Unknown rendering backend reported by Godot".to_string(),
            ),
            RenderBackend::Metal => (
                false,
                "Metal backend is only supported on macOS".to_string(),
            ),
            RenderBackend::D3D12 => (
                false,
                "D3D12 backend is only supported on Windows".to_string(),
            ),
            RenderBackend::Vulkan => (
                false,
                "Vulkan accelerated OSR currently requires x86_64 Windows/Linux hook-based extension injection; hooks are not supported on ARM64".to_string(),
            ),
        }
    }
}

pub fn accelerated_osr_support_diagnostic() -> (bool, String) {
    RenderBackend::detect().accelerated_osr_support_diagnostic()
}

pub use handoff::{AcceleratedInitializationError, AcceleratedRenderState};

#[derive(Clone)]
pub struct AcceleratedRenderHandler {
    pub device_scale_factor: Arc<Mutex<f32>>,
    pub size: Arc<Mutex<cef_app::PhysicalSize<f32>>>,
    pub cursor_type: Arc<Mutex<cef_app::CursorType>>,
    pub popup_state: Arc<Mutex<cef_app::PopupState>>,
    render_state: Option<Arc<Mutex<AcceleratedRenderState>>>,
    repaint_requests: Arc<AtomicU8>,
}

impl AcceleratedRenderHandler {
    pub fn new(device_scale_factor: f32, size: cef_app::PhysicalSize<f32>) -> Self {
        Self {
            device_scale_factor: Arc::new(Mutex::new(device_scale_factor)),
            size: Arc::new(Mutex::new(size)),
            cursor_type: Arc::new(Mutex::new(cef_app::CursorType::default())),
            popup_state: Arc::new(Mutex::new(cef_app::PopupState::new())),
            render_state: None,
            repaint_requests: Arc::new(AtomicU8::new(0)),
        }
    }

    pub fn set_render_state(&mut self, state: Arc<Mutex<AcceleratedRenderState>>) {
        if let Ok(mut state) = state.lock() {
            state.set_repaint_requests(Arc::clone(&self.repaint_requests));
        }
        self.render_state = Some(state);
    }

    pub fn on_accelerated_paint(
        &self,
        type_: PaintElementType,
        info: Option<&AcceleratedPaintInfo>,
    ) {
        let (Some(info), Some(shared)) = (info, &self.render_state) else {
            return;
        };
        let bit = if type_ == PaintElementType::VIEW {
            REPAINT_VIEW
        } else if type_ == PaintElementType::POPUP {
            REPAINT_POPUP
        } else {
            return;
        };
        // Dropping a static page's only paint requires an explicit new paint;
        // external begin-frame alone does not invalidate unchanged content.
        self.repaint_requests.fetch_or(bit, Ordering::Release);
        // Never wait for a future draw or for a slot. All GPU reads of info are
        // completed inside capture; only application-owned snapshots survive.
        let Ok(mut state) = shared.try_lock() else {
            return;
        };
        let mut attempt = RepaintAttempt::claim(&self.repaint_requests, bit);
        attempt.captured = state.capture(type_, info);
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

#[cfg(test)]
mod repaint_tests {
    use super::*;

    #[test]
    fn dropped_capture_keeps_retry_until_a_later_capture_succeeds() {
        let pending = Arc::new(AtomicU8::new(REPAINT_VIEW | REPAINT_POPUP));
        // Bootstrap/full-pool rejection leaves this paint pending.
        drop(RepaintAttempt::claim(&pending, REPAINT_VIEW));
        assert_eq!(
            pending.load(Ordering::Acquire),
            REPAINT_VIEW | REPAINT_POPUP
        );
        // Main-thread invalidation does not consume the pending frame request.
        assert_ne!(pending.load(Ordering::Acquire) & REPAINT_VIEW, 0);
        let mut retry = RepaintAttempt::claim(&pending, REPAINT_VIEW);
        retry.captured = true;
        drop(retry);
        assert_eq!(pending.load(Ordering::Acquire), REPAINT_POPUP);
    }

    #[test]
    fn successful_capture_cannot_clear_a_newer_dropped_paint() {
        let pending = Arc::new(AtomicU8::new(REPAINT_VIEW));
        let mut older = RepaintAttempt::claim(&pending, REPAINT_VIEW);
        // Another callback fails try_lock while the older native copy runs.
        pending.fetch_or(REPAINT_VIEW, Ordering::Release);
        older.captured = true;
        drop(older);
        assert_eq!(pending.load(Ordering::Acquire), REPAINT_VIEW);
    }
}
