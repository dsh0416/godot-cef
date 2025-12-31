use super::{NativeHandleTrait, RenderBackend, SharedTextureInfo, TextureImporterTrait};
use cef::AcceleratedPaintInfo;
use godot::global::godot_warn;
use godot::prelude::*;
use std::ffi::c_void;

pub struct NativeHandle {
    handle: *mut c_void,
}

impl NativeHandle {
    pub fn as_ptr(&self) -> *mut c_void {
        self.handle
    }

    pub fn from_handle(handle: *mut c_void) -> Self {
        Self { handle }
    }
}

impl Default for NativeHandle {
    fn default() -> Self {
        Self {
            handle: std::ptr::null_mut(),
        }
    }
}

impl Clone for NativeHandle {
    fn clone(&self) -> Self {
        Self { handle: self.handle }
    }
}

unsafe impl Send for NativeHandle {}
unsafe impl Sync for NativeHandle {}

impl NativeHandleTrait for NativeHandle {
    fn is_valid(&self) -> bool {
        !self.handle.is_null()
    }

    fn from_accelerated_paint_info(info: &AcceleratedPaintInfo) -> Self {
        Self::from_handle(info.shared_texture_handle)
    }
}

pub struct NativeTextureImporter {
    _placeholder: (),
}

impl NativeTextureImporter {
    pub fn new() -> Option<Self> {
        // TODO: Initialize D3D12 device
        godot_warn!("[AcceleratedOSR/Windows] D3D12 texture import not yet implemented");
        None
    }
}

pub struct GodotTextureImporter {
    _native_importer: NativeTextureImporter,
}

impl TextureImporterTrait for GodotTextureImporter {
    type Handle = NativeHandle;

    fn new() -> Option<Self> {
        let _native_importer = NativeTextureImporter::new()?;
        let render_backend = RenderBackend::detect();

        if !render_backend.supports_accelerated_osr() {
            godot_warn!(
                "[AcceleratedOSR/Windows] Render backend {:?} does not support accelerated OSR",
                render_backend
            );
            return None;
        }

        Some(Self { _native_importer })
    }

    fn import_texture(&mut self, _texture_info: &SharedTextureInfo<Self::Handle>) -> Option<Rid> {
        // TODO: Implement D3D12 texture import
        None
    }

    fn get_color_swap_material(&self) -> Option<Rid> {
        None
    }
}

pub fn is_supported() -> bool {
    false
}
