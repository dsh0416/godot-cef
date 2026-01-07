use super::{NativeHandleTrait, RenderBackend, SharedTextureInfo, TextureImporterTrait};
use cef::AcceleratedPaintInfo;
use godot::classes::rendering_device::DriverResource;
use godot::classes::RenderingServer;
use godot::classes::image::Format as ImageFormat;
use godot::classes::rendering_server::TextureType;
use godot::global::{godot_error, godot_print, godot_warn};
use godot::prelude::*;
use std::ffi::c_void;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Graphics::Direct3D12::{
    D3D12_COMMAND_LIST_TYPE_DIRECT, D3D12_COMMAND_QUEUE_DESC, D3D12_RESOURCE_BARRIER,
    D3D12_RESOURCE_BARRIER_0, D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
    D3D12_RESOURCE_BARRIER_FLAG_NONE, D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
    D3D12_RESOURCE_DESC, D3D12_RESOURCE_DIMENSION_TEXTURE2D, D3D12_RESOURCE_STATE_COMMON,
    D3D12_RESOURCE_STATE_COPY_DEST, D3D12_RESOURCE_STATE_COPY_SOURCE,
    D3D12_RESOURCE_TRANSITION_BARRIER, ID3D12CommandAllocator,
    ID3D12CommandQueue, ID3D12Device, ID3D12Fence, ID3D12GraphicsCommandList, ID3D12Resource,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM,
};
use windows::Win32::Foundation::{CloseHandle, DuplicateHandle, DUPLICATE_SAME_ACCESS};
use windows::Win32::System::Threading::{CreateEventW, GetCurrentProcess, WaitForSingleObject, INFINITE};
use windows::core::Interface;

const COLOR_SWAP_SHADER: &str = r#"
shader_type canvas_item;

void fragment() {
    vec4 tex_color = texture(TEXTURE, UV);
    COLOR = vec4(tex_color.b, tex_color.g, tex_color.r, tex_color.a);
}
"#;

/// Native handle wrapping a Windows HANDLE for D3D12 shared textures.
/// CEF provides this handle for cross-process texture sharing.
/// We duplicate the handle to keep it valid after CEF's on_accelerated_paint returns.
pub struct NativeHandle {
    handle: HANDLE,
    /// Whether this handle was duplicated and needs to be closed on drop
    owned: bool,
}

impl NativeHandle {
    pub fn as_handle(&self) -> HANDLE {
        self.handle
    }

    pub fn as_ptr(&self) -> *mut c_void {
        self.handle.0
    }

    /// Creates a NativeHandle by duplicating the given handle.
    /// This ensures the handle remains valid even after CEF closes its copy.
    pub fn from_handle(handle: *mut c_void) -> Self {
        let source_handle = HANDLE(handle);
        if source_handle.is_invalid() {
            return Self {
                handle: HANDLE::default(),
                owned: false,
            };
        }

        // Duplicate the handle to keep it valid after CEF's callback returns
        let mut duplicated_handle = HANDLE::default();
        let current_process = unsafe { GetCurrentProcess() };
        
        let result = unsafe {
            DuplicateHandle(
                current_process,
                source_handle,
                current_process,
                &mut duplicated_handle,
                0,
                false,
                DUPLICATE_SAME_ACCESS,
            )
        };

        if result.is_ok() && !duplicated_handle.is_invalid() {
            Self {
                handle: duplicated_handle,
                owned: true,
            }
        } else {
            godot_warn!(
                "[AcceleratedOSR/Windows] Failed to duplicate handle, using original (may become invalid)"
            );
            Self {
                handle: source_handle,
                owned: false,
            }
        }
    }
}

impl Default for NativeHandle {
    fn default() -> Self {
        Self {
            handle: HANDLE::default(),
            owned: false,
        }
    }
}

impl Clone for NativeHandle {
    fn clone(&self) -> Self {
        if self.handle.is_invalid() {
            return Self::default();
        }

        // Duplicate the handle for the clone
        let mut duplicated_handle = HANDLE::default();
        let current_process = unsafe { GetCurrentProcess() };
        
        let result = unsafe {
            DuplicateHandle(
                current_process,
                self.handle,
                current_process,
                &mut duplicated_handle,
                0,
                false,
                DUPLICATE_SAME_ACCESS,
            )
        };

        if result.is_ok() && !duplicated_handle.is_invalid() {
            Self {
                handle: duplicated_handle,
                owned: true,
            }
        } else {
            // Fallback: share the handle without owning it
            Self {
                handle: self.handle,
                owned: false,
            }
        }
    }
}

impl Drop for NativeHandle {
    fn drop(&mut self) {
        if self.owned && !self.handle.is_invalid() {
            let _ = unsafe { CloseHandle(self.handle) };
            self.handle = HANDLE::default();
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
/// Uses Godot's D3D12 device obtained via RenderingDevice::get_driver_resource()
/// to ensure resource compatibility for GPU copy operations.
pub struct NativeTextureImporter {
    /// D3D12 device borrowed from Godot - wrapped in ManuallyDrop to prevent Release
    device: std::mem::ManuallyDrop<ID3D12Device>,
    /// Command queue borrowed from Godot - wrapped in ManuallyDrop to prevent Release
    command_queue: std::mem::ManuallyDrop<ID3D12CommandQueue>,
    /// Command allocator - owned by us
    command_allocator: ID3D12CommandAllocator,
    /// Fence for synchronization - owned by us
    fence: ID3D12Fence,
    fence_value: u64,
    fence_event: HANDLE,
}

impl NativeTextureImporter {
    pub fn new() -> Option<Self> {
        let mut rd = RenderingServer::singleton()
            .get_rendering_device()
            .ok_or_else(|| {
                godot_error!("[AcceleratedOSR/Windows] Failed to get RenderingDevice");
            })
            .ok()?;

        let device_ptr = rd.get_driver_resource(
            DriverResource::LOGICAL_DEVICE,
            Rid::Invalid,
            0,
        );

        if device_ptr == 0 {
            godot_error!("[AcceleratedOSR/Windows] Failed to get D3D12 device from Godot");
            return None;
        }

        let device: ID3D12Device = unsafe {
            ID3D12Device::from_raw(device_ptr as *mut c_void)
        };

        // Get the command queue from Godot
        let command_queue_ptr = rd.get_driver_resource(
            DriverResource::COMMAND_QUEUE,
            Rid::Invalid,
            0,
        );

        let command_queue: ID3D12CommandQueue = if command_queue_ptr != 0 {
            unsafe { ID3D12CommandQueue::from_raw(command_queue_ptr as *mut c_void) }
        } else {
            // Fallback: create our own command queue using Godot's device
            godot_warn!("[AcceleratedOSR/Windows] Could not get command queue from Godot, creating one");
            let queue_desc = D3D12_COMMAND_QUEUE_DESC {
                Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
                ..Default::default()
            };
            unsafe { device.CreateCommandQueue(&queue_desc) }
                .map_err(|e| {
                    godot_error!(
                        "[AcceleratedOSR/Windows] Failed to create command queue: {:?}",
                        e
                    )
                })
                .ok()?
        };

        // Create command allocator using Godot's device
        let command_allocator: ID3D12CommandAllocator =
            unsafe { device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT) }
                .map_err(|e| {
                    godot_error!(
                        "[AcceleratedOSR/Windows] Failed to create command allocator: {:?}",
                        e
                    )
                })
                .ok()?;

        // Create fence for synchronization
        let fence: ID3D12Fence = unsafe { device.CreateFence(0, windows::Win32::Graphics::Direct3D12::D3D12_FENCE_FLAG_NONE) }
            .map_err(|e| {
                godot_error!(
                    "[AcceleratedOSR/Windows] Failed to create fence: {:?}",
                    e
                )
            })
            .ok()?;

        let fence_event = unsafe { CreateEventW(None, false, false, None) }
            .map_err(|e| {
                godot_error!(
                    "[AcceleratedOSR/Windows] Failed to create fence event: {:?}",
                    e
                )
            })
            .ok()?;

        godot_print!(
            "[AcceleratedOSR/Windows] Using Godot's D3D12 device for accelerated OSR"
        );

        Some(Self {
            device: std::mem::ManuallyDrop::new(device),
            command_queue: std::mem::ManuallyDrop::new(command_queue),
            command_allocator,
            fence,
            fence_value: 0,
            fence_event,
        })
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
        unsafe { self.device.OpenSharedHandle(handle, &mut resource) }
            .map_err(|e| format!("Failed to open shared handle: {:?}", e))?;

        let resource =
            resource.ok_or_else(|| "OpenSharedHandle returned null resource".to_string())?;

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

    /// Copies from a source D3D12 resource to a destination D3D12 resource.
    pub fn copy_texture(
        &mut self,
        src_resource: &ID3D12Resource,
        dst_resource: &ID3D12Resource,
    ) -> Result<(), String> {
        // Reset command allocator
        unsafe { self.command_allocator.Reset() }
            .map_err(|e| format!("Failed to reset command allocator: {:?}", e))?;

        // Create command list
        let command_list: ID3D12GraphicsCommandList = unsafe {
            self.device.CreateCommandList(
                0,
                D3D12_COMMAND_LIST_TYPE_DIRECT,
                &self.command_allocator,
                None,
            )
        }
        .map_err(|e| format!("Failed to create command list: {:?}", e))?;

        // Transition source to COPY_SOURCE state
        let src_barrier = D3D12_RESOURCE_BARRIER {
            Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            Anonymous: D3D12_RESOURCE_BARRIER_0 {
                Transition: std::mem::ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                    pResource: unsafe { std::mem::transmute_copy(src_resource) },
                    Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                    StateBefore: D3D12_RESOURCE_STATE_COMMON,
                    StateAfter: D3D12_RESOURCE_STATE_COPY_SOURCE,
                }),
            },
        };

        // Transition destination to COPY_DEST state
        let dst_barrier = D3D12_RESOURCE_BARRIER {
            Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            Anonymous: D3D12_RESOURCE_BARRIER_0 {
                Transition: std::mem::ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                    pResource: unsafe { std::mem::transmute_copy(dst_resource) },
                    Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                    StateBefore: D3D12_RESOURCE_STATE_COMMON,
                    StateAfter: D3D12_RESOURCE_STATE_COPY_DEST,
                }),
            },
        };

        let barriers_before = [src_barrier, dst_barrier];
        unsafe { command_list.ResourceBarrier(&barriers_before) };

        // Copy the resource
        unsafe { command_list.CopyResource(dst_resource, src_resource) };

        // Transition resources back to COMMON state
        let src_barrier_after = D3D12_RESOURCE_BARRIER {
            Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            Anonymous: D3D12_RESOURCE_BARRIER_0 {
                Transition: std::mem::ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                    pResource: unsafe { std::mem::transmute_copy(src_resource) },
                    Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                    StateBefore: D3D12_RESOURCE_STATE_COPY_SOURCE,
                    StateAfter: D3D12_RESOURCE_STATE_COMMON,
                }),
            },
        };

        let dst_barrier_after = D3D12_RESOURCE_BARRIER {
            Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            Anonymous: D3D12_RESOURCE_BARRIER_0 {
                Transition: std::mem::ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                    pResource: unsafe { std::mem::transmute_copy(dst_resource) },
                    Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                    StateBefore: D3D12_RESOURCE_STATE_COPY_DEST,
                    StateAfter: D3D12_RESOURCE_STATE_COMMON,
                }),
            },
        };

        let barriers_after = [src_barrier_after, dst_barrier_after];
        unsafe { command_list.ResourceBarrier(&barriers_after) };

        // Close and execute command list
        unsafe { command_list.Close() }
            .map_err(|e| format!("Failed to close command list: {:?}", e))?;

        let command_lists = [Some(command_list.cast::<windows::Win32::Graphics::Direct3D12::ID3D12CommandList>().unwrap())];
        unsafe { self.command_queue.ExecuteCommandLists(&command_lists) };

        // Signal and wait for completion
        self.fence_value += 1;
        unsafe { self.command_queue.Signal(&self.fence, self.fence_value) }
            .map_err(|e| format!("Failed to signal fence: {:?}", e))?;

        if unsafe { self.fence.GetCompletedValue() } < self.fence_value {
            unsafe { self.fence.SetEventOnCompletion(self.fence_value, self.fence_event) }
                .map_err(|e| format!("Failed to set event on completion: {:?}", e))?;
            unsafe { WaitForSingleObject(self.fence_event, INFINITE) };
        }

        Ok(())
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
            "[AcceleratedOSR/Windows] Using Godot's D3D12 backend for texture import"
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

    fn copy_texture(
        &mut self,
        src_info: &SharedTextureInfo<Self::Handle>,
        dst_rd_rid: Rid,
    ) -> Result<(), String> {
        let handle = src_info.native_handle().as_handle();
        if handle.is_invalid() {
            return Err("Source handle is invalid".into());
        }
        if src_info.width == 0 || src_info.height == 0 {
            return Err(format!(
                "Invalid source dimensions: {}x{}",
                src_info.width, src_info.height
            ));
        }
        if !dst_rd_rid.is_valid() {
            return Err("Destination RID is invalid".into());
        }

        // Import the shared handle into a D3D12 resource (source)
        let src_resource = self.d3d12_importer.import_shared_handle(
            handle,
            src_info.width,
            src_info.height,
            src_info.format,
        )?;

        // Get destination D3D12 resource from Godot's RenderingDevice
        let dst_resource = {
            let mut rd = RenderingServer::singleton()
                .get_rendering_device()
                .ok_or("Failed to get RenderingDevice")?;

            let resource_ptr = rd.get_driver_resource(
                DriverResource::TEXTURE,
                dst_rd_rid,
                0,
            );

            if resource_ptr == 0 {
                return Err("Failed to get destination D3D12 resource handle".into());
            }

            unsafe { ID3D12Resource::from_raw(resource_ptr as *mut c_void) }
        };

        self.d3d12_importer.copy_texture(&src_resource, &dst_resource)?;
        std::mem::forget(dst_resource);

        Ok(())
    }

    fn get_color_swap_material(&self) -> Option<Rid> {
        self.color_swap_material
    }
}

impl Drop for NativeTextureImporter {
    fn drop(&mut self) {
        if !self.fence_event.is_invalid() {
            let _ = unsafe { CloseHandle(self.fence_event) };
        }
        // device and command_queue are ManuallyDrop (borrowed from Godot)
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
