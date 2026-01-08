use super::{NativeHandleTrait, RenderBackend, SharedTextureInfo, TextureImporterTrait};
use cef::AcceleratedPaintInfo;
use godot::global::godot_warn;
use godot::prelude::*;

pub struct NativeHandle {
    fd: i32,
}

/// Not implemented yet
#[allow(dead_code)]
impl NativeHandle {
    pub fn fd(&self) -> i32 {
        self.fd
    }

    pub fn from_fd(fd: i32) -> Self {
        Self {
            fd: if fd < 0 { -1 } else { fd },
        }
    }
}

impl Default for NativeHandle {
    fn default() -> Self {
        Self { fd: -1 }
    }
}

impl Clone for NativeHandle {
    fn clone(&self) -> Self {
        Self { fd: self.fd }
    }
}

impl Drop for NativeHandle {
    fn drop(&mut self) {
        if self.fd >= 0 {
            // TODO: libc::close(self.fd);
            self.fd = -1;
        }
    }
}

unsafe impl Send for NativeHandle {}
unsafe impl Sync for NativeHandle {}

impl NativeHandleTrait for NativeHandle {
    fn is_valid(&self) -> bool {
        self.fd >= 0
    }

    fn from_accelerated_paint_info(_info: &AcceleratedPaintInfo) -> Self {
        // TODO: Extract DMA-BUF fd from CEF
        Self::default()
    }
}

pub struct NativeTextureImporter {
    _placeholder: (),
}

impl NativeTextureImporter {
    pub fn new() -> Option<Self> {
        // TODO: Initialize Vulkan with external memory extensions
        godot_warn!("[AcceleratedOSR/Linux] Vulkan texture import not yet implemented");
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
                "[AcceleratedOSR/Linux] Render backend {:?} does not support accelerated OSR",
                render_backend
            );
            return None;
        }

        Some(Self { _native_importer })
    }

    fn copy_texture(
        &mut self,
        _src_info: &SharedTextureInfo<Self::Handle>,
        _dst_rd_rid: Rid,
    ) -> Result<(), String> {
        // TODO: Implement Vulkan texture copy
        Err("Accelerated OSR not yet implemented on Linux".to_string())
    }
}

pub fn is_supported() -> bool {
    false
}
