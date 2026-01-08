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

pub struct SharedTextureInfo<H: NativeHandleTrait> {
    native_handle: H,
    pub width: u32,
    pub height: u32,
    pub format: cef::sys::cef_color_type_t,
    pub dirty: bool,
    pub frame_count: u64,
}

impl<H: NativeHandleTrait> SharedTextureInfo<H> {
    pub fn native_handle(&self) -> &H {
        &self.native_handle
    }

    pub fn set_native_handle(&mut self, new_handle: H) {
        self.native_handle = new_handle;
    }
}

impl<H: NativeHandleTrait + Default> Default for SharedTextureInfo<H> {
    fn default() -> Self {
        Self {
            native_handle: H::default(),
            width: 0,
            height: 0,
            format: cef::sys::cef_color_type_t::CEF_COLOR_TYPE_BGRA_8888,
            dirty: false,
            frame_count: 0,
        }
    }
}

impl<H: NativeHandleTrait + Clone> Clone for SharedTextureInfo<H> {
    fn clone(&self) -> Self {
        Self {
            native_handle: self.native_handle.clone(),
            width: self.width,
            height: self.height,
            format: self.format,
            dirty: self.dirty,
            frame_count: self.frame_count,
        }
    }
}

unsafe impl<H: NativeHandleTrait + Send> Send for SharedTextureInfo<H> {}
unsafe impl<H: NativeHandleTrait + Sync> Sync for SharedTextureInfo<H> {}

pub trait NativeHandleTrait: Sized {
    fn is_valid(&self) -> bool;
    fn from_accelerated_paint_info(info: &AcceleratedPaintInfo) -> Self;
}

pub trait TextureImporterTrait {
    type Handle: NativeHandleTrait;

    fn new() -> Option<Self>
    where
        Self: Sized;

    /// Copies the CEF shared texture to a Godot-owned texture via GPU-to-GPU copy.
    ///
    /// # Arguments
    /// * `src_info` - The source texture info from CEF
    /// * `dst_rd_rid` - The RenderingDevice RID of the destination Godot texture (obtained via RenderingServer::texture_get_rd_texture)
    ///
    /// # Returns
    /// * `Ok(())` on successful copy
    /// * `Err(String)` with error description on failure
    fn copy_texture(
        &mut self,
        src_info: &SharedTextureInfo<Self::Handle>,
        dst_rd_rid: Rid,
    ) -> Result<(), String>;
}

pub struct AcceleratedRenderHandler<H: NativeHandleTrait + Default + Send + Sync + 'static> {
    pub texture_info: Arc<Mutex<SharedTextureInfo<H>>>,
    pub device_scale_factor: Arc<Mutex<f32>>,
    pub size: Arc<Mutex<winit::dpi::PhysicalSize<f32>>>,
    pub cursor_type: Arc<Mutex<cef_app::CursorType>>,
}

impl<H: NativeHandleTrait + Default + Clone + Send + Sync + 'static> Clone
    for AcceleratedRenderHandler<H>
{
    fn clone(&self) -> Self {
        Self {
            texture_info: self.texture_info.clone(),
            device_scale_factor: self.device_scale_factor.clone(),
            size: self.size.clone(),
            cursor_type: self.cursor_type.clone(),
        }
    }
}

impl<H: NativeHandleTrait + Default + Clone + Send + Sync + 'static> AcceleratedRenderHandler<H> {
    pub fn new(device_scale_factor: f32, size: winit::dpi::PhysicalSize<f32>) -> Self {
        Self {
            texture_info: Arc::new(Mutex::new(SharedTextureInfo::default())),
            device_scale_factor: Arc::new(Mutex::new(device_scale_factor)),
            size: Arc::new(Mutex::new(size)),
            cursor_type: Arc::new(Mutex::new(cef_app::CursorType::default())),
        }
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

        if let Ok(mut tex_info) = self.texture_info.lock() {
            tex_info.frame_count += 1;
            tex_info.set_native_handle(H::from_accelerated_paint_info(info));
            tex_info.width = info.extra.coded_size.width as u32;
            tex_info.height = info.extra.coded_size.height as u32;
            tex_info.format = *info.format.as_ref();
            tex_info.dirty = true;
        } else {
            godot::global::godot_error!("[AcceleratedOSR] Failed to lock texture_info");
        }
    }

    pub fn get_texture_info(&self) -> Arc<Mutex<SharedTextureInfo<H>>> {
        self.texture_info.clone()
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

#[cfg(target_os = "macos")]
pub type PlatformSharedTextureInfo = SharedTextureInfo<macos::NativeHandle>;
#[cfg(target_os = "windows")]
pub type PlatformSharedTextureInfo = SharedTextureInfo<windows::NativeHandle>;
#[cfg(target_os = "linux")]
pub type PlatformSharedTextureInfo = SharedTextureInfo<linux::NativeHandle>;
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub type PlatformSharedTextureInfo = SharedTextureInfo<()>;

#[cfg(target_os = "macos")]
pub type PlatformAcceleratedRenderHandler = AcceleratedRenderHandler<macos::NativeHandle>;
#[cfg(target_os = "windows")]
pub type PlatformAcceleratedRenderHandler = AcceleratedRenderHandler<windows::NativeHandle>;
#[cfg(target_os = "linux")]
pub type PlatformAcceleratedRenderHandler = AcceleratedRenderHandler<linux::NativeHandle>;
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub type PlatformAcceleratedRenderHandler = AcceleratedRenderHandler<()>;

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
impl NativeHandleTrait for () {
    fn is_valid(&self) -> bool {
        false
    }

    fn from_accelerated_paint_info(_info: &AcceleratedPaintInfo) -> Self {}
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub struct GodotTextureImporter;

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
impl TextureImporterTrait for GodotTextureImporter {
    type Handle = ();

    fn new() -> Option<Self> {
        None
    }

    fn copy_texture(
        &mut self,
        _src_info: &SharedTextureInfo<Self::Handle>,
        _dst_rd_rid: Rid,
    ) -> Result<(), String> {
        Err("Accelerated OSR not supported on this platform".to_string())
    }
}
