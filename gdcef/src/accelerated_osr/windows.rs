use super::{NativeHandleTrait, RenderBackend, SharedTextureInfo, TextureImporterTrait};
use cef::AcceleratedPaintInfo;
use godot::classes::image::Format as ImageFormat;
use godot::classes::rendering_server::TextureType;
use godot::classes::RenderingServer;
use godot::global::{godot_error, godot_print, godot_warn};
use godot::prelude::*;
use std::ffi::c_void;
use windows::core::Interface;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_12_0;
use windows::Win32::Graphics::Direct3D12::{
    D3D12CreateDevice, ID3D12Device, ID3D12Resource, D3D12_RESOURCE_DESC,
    D3D12_RESOURCE_DIMENSION_TEXTURE2D,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, IDXGIAdapter1, IDXGIFactory4, DXGI_ADAPTER_FLAG,
    DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_CREATE_FACTORY_FLAGS,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM,
};

const COLOR_SWAP_SHADER: &str = r#"
shader_type canvas_item;

void fragment() {
    vec4 tex_color = texture(TEXTURE, UV);
    COLOR = vec4(tex_color.b, tex_color.g, tex_color.r, tex_color.a);
}
"#;

/// Native handle wrapping a Windows HANDLE for D3D12 shared textures.
/// CEF provides this handle for cross-process texture sharing.
pub struct NativeHandle {
    handle: HANDLE,
}

impl NativeHandle {
    pub fn as_handle(&self) -> HANDLE {
        self.handle
    }

    pub fn as_ptr(&self) -> *mut c_void {
        self.handle.0
    }

    pub fn from_handle(handle: *mut c_void) -> Self {
        Self {
            handle: HANDLE(handle),
        }
    }
}

impl Default for NativeHandle {
    fn default() -> Self {
        Self {
            handle: HANDLE::default(),
        }
    }
}

impl Clone for NativeHandle {
    fn clone(&self) -> Self {
        // Shared handles are reference-counted by the OS and don't need explicit retain
        Self {
            handle: self.handle,
        }
    }
}

unsafe impl Send for NativeHandle {}
unsafe impl Sync for NativeHandle {}

impl NativeHandleTrait for NativeHandle {
    fn is_valid(&self) -> bool {
        !self.handle.is_invalid()
    }

    fn from_accelerated_paint_info(info: &AcceleratedPaintInfo) -> Self {
        Self::from_handle(info.shared_texture_handle)
    }
}

/// D3D12 device and resources for importing shared textures from CEF.
pub struct NativeTextureImporter {
    device: ID3D12Device,
    adapter_name: String,
}

impl NativeTextureImporter {
    pub fn new() -> Option<Self> {
        // Create DXGI factory
        let factory: IDXGIFactory4 = unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)) }
            .map_err(|e| godot_error!("[AcceleratedOSR/Windows] Failed to create DXGI factory: {:?}", e))
            .ok()?;

        // Find a suitable hardware adapter
        let adapter = Self::get_hardware_adapter(&factory)?;
        let adapter_desc = unsafe { adapter.GetDesc1() }
            .map_err(|e| godot_error!("[AcceleratedOSR/Windows] Failed to get adapter description: {:?}", e))
            .ok()?;

        let adapter_name = String::from_utf16_lossy(
            &adapter_desc.Description[..adapter_desc.Description.iter().position(|&c| c == 0).unwrap_or(adapter_desc.Description.len())]
        );

        // Create D3D12 device
        let mut device: Option<ID3D12Device> = None;
        unsafe {
            D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_12_0, &mut device)
        }
        .map_err(|e| godot_error!("[AcceleratedOSR/Windows] Failed to create D3D12 device: {:?}", e))
        .ok()?;

        let device = device?;
        godot_print!("[AcceleratedOSR/Windows] Created D3D12 device on adapter: {}", adapter_name);

        Some(Self { device, adapter_name })
    }

    fn get_hardware_adapter(factory: &IDXGIFactory4) -> Option<IDXGIAdapter1> {
        for i in 0.. {
            let adapter: IDXGIAdapter1 = match unsafe { factory.EnumAdapters1(i) } {
                Ok(adapter) => adapter,
                Err(_) => break,
            };

            let desc = match unsafe { adapter.GetDesc1() } {
                Ok(desc) => desc,
                Err(_) => continue,
            };

            // Skip software adapters
            if (DXGI_ADAPTER_FLAG(desc.Flags as i32) & DXGI_ADAPTER_FLAG_SOFTWARE)
                != DXGI_ADAPTER_FLAG(0)
            {
                continue;
            }

            // Check if the adapter supports D3D12
            let result = unsafe {
                D3D12CreateDevice(
                    &adapter,
                    D3D_FEATURE_LEVEL_12_0,
                    std::ptr::null_mut::<Option<ID3D12Device>>(),
                )
            };

            if result.is_ok() {
                return Some(adapter);
            }
        }

        godot_warn!("[AcceleratedOSR/Windows] No suitable D3D12 hardware adapter found");
        None
    }

    /// Import a shared texture handle from CEF into a D3D12 resource.
    ///
    /// CEF shares textures via NT handles (created with D3D12_RESOURCE_FLAG_ALLOW_CROSS_ADAPTER
    /// or similar sharing flags). We open this handle to get access to the texture.
    pub fn import_shared_handle(
        &self,
        handle: HANDLE,
        _width: u32,
        _height: u32,
        format: cef::sys::cef_color_type_t,
    ) -> Result<ID3D12Resource, String> {
        if handle.is_invalid() {
            return Err("Shared handle is invalid".into());
        }

        // Determine expected DXGI format based on CEF color type
        let _expected_format = match format {
            cef::sys::cef_color_type_t::CEF_COLOR_TYPE_RGBA_8888 => DXGI_FORMAT_R8G8B8A8_UNORM,
            _ => DXGI_FORMAT_B8G8R8A8_UNORM,
        };

        // Open the shared handle to get the D3D12 resource
        let mut resource: Option<ID3D12Resource> = None;
        unsafe {
            self.device.OpenSharedHandle(handle, &mut resource)
        }.map_err(|e| format!("Failed to open shared handle: {:?}", e))?;

        let resource = resource.ok_or_else(|| "OpenSharedHandle returned null resource".to_string())?;

        // Validate the resource description
        let desc: D3D12_RESOURCE_DESC = unsafe { resource.GetDesc() };
        if desc.Dimension != D3D12_RESOURCE_DIMENSION_TEXTURE2D {
            return Err(format!(
                "Expected 2D texture, got dimension {:?}",
                desc.Dimension
            ));
        }

        Ok(resource)
    }

    pub fn device(&self) -> &ID3D12Device {
        &self.device
    }

    pub fn adapter_name(&self) -> &str {
        &self.adapter_name
    }
}

/// Imports D3D12 shared textures from CEF into Godot's rendering system.
pub struct GodotTextureImporter {
    d3d12_importer: NativeTextureImporter,
    current_d3d12_resource: Option<ID3D12Resource>,
    current_texture_rid: Option<Rid>,
    color_swap_shader: Option<Rid>,
    color_swap_material: Option<Rid>,
}

impl TextureImporterTrait for GodotTextureImporter {
    type Handle = NativeHandle;

    fn new() -> Option<Self> {
        let d3d12_importer = NativeTextureImporter::new()?;
        let render_backend = RenderBackend::detect();

        if !render_backend.supports_accelerated_osr() {
            godot_warn!(
                "[AcceleratedOSR/Windows] Render backend {:?} does not support accelerated OSR. \
                 D3D12 backend is required on Windows.",
                render_backend
            );
            return None;
        }

        godot_print!(
            "[AcceleratedOSR/Windows] Using D3D12 backend for texture import (adapter: {})",
            d3d12_importer.adapter_name()
        );

        // Create color swap shader for BGRA -> RGBA conversion if needed
        let mut rs = RenderingServer::singleton();
        let shader_rid = rs.shader_create();
        rs.shader_set_code(shader_rid, COLOR_SWAP_SHADER);
        let material_rid = rs.material_create();
        rs.material_set_shader(material_rid, shader_rid);

        Some(Self {
            d3d12_importer,
            current_d3d12_resource: None,
            current_texture_rid: None,
            color_swap_shader: Some(shader_rid),
            color_swap_material: Some(material_rid),
        })
    }

    fn import_texture(&mut self, texture_info: &SharedTextureInfo<Self::Handle>) -> Option<Rid> {
        let handle = texture_info.native_handle().as_handle();
        if handle.is_invalid() || texture_info.width == 0 || texture_info.height == 0 {
            return None;
        }

        // Import the shared handle into a D3D12 resource
        let d3d12_resource = self
            .d3d12_importer
            .import_shared_handle(
                handle,
                texture_info.width,
                texture_info.height,
                texture_info.format,
            )
            .map_err(|e| godot_error!("[AcceleratedOSR/Windows] D3D12 import failed: {}", e))
            .ok()?;

        // Free the previous Godot texture RID
        if let Some(old_rid) = self.current_texture_rid.take() {
            RenderingServer::singleton().free_rid(old_rid);
        }

        // Store the new D3D12 resource (previous one will be dropped automatically)
        self.current_d3d12_resource = Some(d3d12_resource.clone());

        // Get the native handle as a u64 pointer for Godot
        // For D3D12, we pass the ID3D12Resource pointer
        let native_handle = d3d12_resource.as_raw() as u64;

        // Determine the image format based on CEF's color type
        let image_format = match texture_info.format {
            cef::sys::cef_color_type_t::CEF_COLOR_TYPE_RGBA_8888 => ImageFormat::RGBA8,
            _ => ImageFormat::RGBA8, // Godot expects RGBA8, shader will swap if BGRA
        };

        // Create Godot texture from native D3D12 resource handle
        let texture_rid = RenderingServer::singleton().texture_create_from_native_handle(
            TextureType::TYPE_2D,
            image_format,
            native_handle,
            texture_info.width as i32,
            texture_info.height as i32,
            1, // layers
        );

        if !texture_rid.is_valid() {
            godot_error!("[AcceleratedOSR/Windows] Created texture RID is invalid");
            return None;
        }

        self.current_texture_rid = Some(texture_rid);
        Some(texture_rid)
    }

    fn get_color_swap_material(&self) -> Option<Rid> {
        self.color_swap_material
    }
}

impl Drop for GodotTextureImporter {
    fn drop(&mut self) {
        let mut rs = RenderingServer::singleton();

        // Free Godot resources
        if let Some(rid) = self.current_texture_rid.take() {
            rs.free_rid(rid);
        }

        // D3D12 resource will be dropped automatically via the COM Release mechanism

        if let Some(rid) = self.color_swap_material.take() {
            rs.free_rid(rid);
        }
        if let Some(rid) = self.color_swap_shader.take() {
            rs.free_rid(rid);
        }
    }
}

pub fn is_supported() -> bool {
    NativeTextureImporter::new().is_some() && RenderBackend::detect().supports_accelerated_osr()
}
