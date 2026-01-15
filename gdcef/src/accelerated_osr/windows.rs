use super::RenderBackend;
use godot::classes::RenderingServer;
use godot::classes::rendering_device::DriverResource;
use godot::global::{godot_error, godot_print, godot_warn};
use godot::prelude::*;
use std::ffi::c_void;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Graphics::Direct3D12::{
    D3D12_COMMAND_LIST_TYPE_DIRECT, D3D12_COMMAND_QUEUE_DESC, D3D12_RESOURCE_BARRIER,
    D3D12_RESOURCE_BARRIER_0, D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
    D3D12_RESOURCE_BARRIER_FLAG_NONE, D3D12_RESOURCE_BARRIER_TYPE_TRANSITION, D3D12_RESOURCE_DESC,
    D3D12_RESOURCE_DIMENSION_TEXTURE2D, D3D12_RESOURCE_STATE_COMMON,
    D3D12_RESOURCE_STATE_COPY_DEST, D3D12_RESOURCE_TRANSITION_BARRIER, ID3D12CommandAllocator,
    ID3D12CommandQueue, ID3D12Device, ID3D12Fence, ID3D12GraphicsCommandList, ID3D12Resource,
};
use windows::Win32::System::Threading::{CreateEventW, INFINITE, WaitForSingleObject};
use windows::core::Interface;

pub struct NativeTextureImporter {
    device: std::mem::ManuallyDrop<ID3D12Device>,
    command_queue: ID3D12CommandQueue,
    command_allocator: ID3D12CommandAllocator,
    fence: ID3D12Fence,
    fence_value: u64,
    fence_event: HANDLE,
    pending_copies: std::collections::HashMap<u64, u64>,
    device_removed_logged: bool,
}

impl NativeTextureImporter {
    pub fn new() -> Option<Self> {
        let mut rd = RenderingServer::singleton()
            .get_rendering_device()
            .ok_or_else(|| {
                godot_error!("[AcceleratedOSR/Windows] Failed to get RenderingDevice");
            })
            .ok()?;

        let device_ptr = rd.get_driver_resource(DriverResource::LOGICAL_DEVICE, Rid::Invalid, 0);

        if device_ptr == 0 {
            godot_error!("[AcceleratedOSR/Windows] Failed to get D3D12 device from Godot");
            return None;
        }

        let device: ID3D12Device = unsafe { ID3D12Device::from_raw(device_ptr as *mut c_void) };

        // CRITICAL: Create our OWN command queue instead of using Godot's.
        // Using Godot's command queue causes synchronization conflicts because:
        // 1. Godot is also submitting commands to that queue
        // 2. Our fence signals don't synchronize with Godot's operations
        // 3. This causes DEVICE_HUNG errors on the second frame
        let queue_desc = D3D12_COMMAND_QUEUE_DESC {
            Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
            ..Default::default()
        };
        let command_queue: ID3D12CommandQueue = unsafe { device.CreateCommandQueue(&queue_desc) }
            .map_err(|e| {
                godot_error!(
                    "[AcceleratedOSR/Windows] Failed to create command queue: {:?}",
                    e
                )
            })
            .ok()?;

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
        let fence: ID3D12Fence = unsafe {
            device.CreateFence(
                0,
                windows::Win32::Graphics::Direct3D12::D3D12_FENCE_FLAG_NONE,
            )
        }
        .map_err(|e| godot_error!("[AcceleratedOSR/Windows] Failed to create fence: {:?}", e))
        .ok()?;

        let fence_event = unsafe { CreateEventW(None, false, false, None) }
            .map_err(|e| {
                godot_error!(
                    "[AcceleratedOSR/Windows] Failed to create fence event: {:?}",
                    e
                )
            })
            .ok()?;

        godot_print!("[AcceleratedOSR/Windows] Using Godot's D3D12 device for accelerated OSR");

        Some(Self {
            device: std::mem::ManuallyDrop::new(device),
            command_queue,
            command_allocator,
            fence,
            fence_value: 0,
            fence_event,
            pending_copies: std::collections::HashMap::new(),
            device_removed_logged: false,
        })
    }

    pub fn check_device_state(&mut self) -> Result<(), String> {
        let reason = unsafe { self.device.GetDeviceRemovedReason() };
        if reason.is_ok() {
            self.device_removed_logged = false;
            Ok(())
        } else if !self.device_removed_logged {
            godot_warn!(
                "[AcceleratedOSR/Windows] D3D12 device removed: {:?}",
                reason.err()
            );
            self.device_removed_logged = true;
            Err("D3D12 device removed".into())
        } else {
            Err("D3D12 device removed".into())
        }
    }

    pub fn import_shared_handle(
        &mut self,
        handle: HANDLE,
        _width: u32,
        _height: u32,
        _format: cef::sys::cef_color_type_t,
    ) -> Result<ID3D12Resource, String> {
        if handle.is_invalid() {
            return Err("Shared handle is invalid".into());
        }

        // Open the shared handle to get the D3D12 resource
        let mut resource: Option<ID3D12Resource> = None;
        let result = unsafe { self.device.OpenSharedHandle(handle, &mut resource) };

        if let Err(e) = result {
            let device_reason = unsafe { self.device.GetDeviceRemovedReason() };
            if !self.device_removed_logged {
                if device_reason.is_err() {
                    godot_warn!(
                        "[AcceleratedOSR/Windows] Device removed: {:?}",
                        device_reason.err()
                    );
                } else {
                    godot_warn!("[AcceleratedOSR/Windows] OpenSharedHandle failed: {:?}", e);
                }
                self.device_removed_logged = true;
            }
            return Err("D3D12 device removed".into());
        }

        self.device_removed_logged = false;

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

    pub fn queue_copy_texture(
        &mut self,
        src_resource: &ID3D12Resource,
        dst_resource: &ID3D12Resource,
    ) -> Result<u64, String> {
        // Wait for previous copy before reusing command allocator
        if self.fence_value > 0 {
            let completed = unsafe { self.fence.GetCompletedValue() };
            if completed < self.fence_value {
                unsafe {
                    self.fence
                        .SetEventOnCompletion(self.fence_value, self.fence_event)
                }
                .map_err(|e| format!("Failed to set event on completion: {:?}", e))?;
                unsafe { WaitForSingleObject(self.fence_event, INFINITE) };
            }
        }

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

        // Transition only the destination to COPY_DEST.
        //
        // The source texture is created and fully managed by CEF. CEF keeps the
        // resource in a state suitable for external consumers (typically COMMON)
        // and expects clients not to perform their own state transitions on it.
        // The previous implementation transitioned the source to COPY_SOURCE and
        // back to COMMON, but that interfered with CEF's own resource state
        // tracking. We now rely on CEF's guarantees and leave the source state
        // untouched, transitioning just our destination resource for the copy.
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

        unsafe { command_list.ResourceBarrier(&[dst_barrier]) };
        unsafe { command_list.CopyResource(dst_resource, src_resource) };

        // Transition back to COMMON for shader read
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

        unsafe { command_list.ResourceBarrier(&[dst_barrier_after]) };

        // Close and execute command list
        unsafe { command_list.Close() }
            .map_err(|e| format!("Failed to close command list: {:?}", e))?;

        let command_lists = [Some(
            command_list
                .cast::<windows::Win32::Graphics::Direct3D12::ID3D12CommandList>()
                .unwrap(),
        )];
        unsafe { self.command_queue.ExecuteCommandLists(&command_lists) };

        self.fence_value += 1;
        unsafe { self.command_queue.Signal(&self.fence, self.fence_value) }
            .map_err(|e| format!("Failed to signal fence: {:?}", e))?;

        let copy_id = self.fence_value;
        self.pending_copies.insert(copy_id, self.fence_value);

        Ok(copy_id)
    }

    pub fn is_copy_complete(&self, copy_id: u64) -> bool {
        let completed_value = unsafe { self.fence.GetCompletedValue() };
        copy_id <= completed_value
    }

    pub fn wait_for_all_copies(&self) {
        if self.fence_value == 0 {
            return;
        }

        let completed = unsafe { self.fence.GetCompletedValue() };
        if completed < self.fence_value {
            let result = unsafe {
                self.fence
                    .SetEventOnCompletion(self.fence_value, self.fence_event)
            };
            if result.is_ok() {
                unsafe { WaitForSingleObject(self.fence_event, INFINITE) };
            }
        }
    }

    pub fn wait_for_copy(&self, copy_id: u64) -> Result<(), String> {
        let completed = unsafe { self.fence.GetCompletedValue() };
        if completed >= copy_id {
            return Ok(());
        }

        unsafe { self.fence.SetEventOnCompletion(copy_id, self.fence_event) }
            .map_err(|e| format!("Failed to set event on completion: {:?}", e))?;

        unsafe { WaitForSingleObject(self.fence_event, INFINITE) };
        Ok(())
    }
}

pub struct GodotTextureImporter {
    d3d12_importer: NativeTextureImporter,
    current_texture_rid: Option<Rid>,
}

impl GodotTextureImporter {
    pub fn new() -> Option<Self> {
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

        godot_print!("[AcceleratedOSR/Windows] Using Godot's D3D12 backend for texture import");

        Some(Self {
            d3d12_importer,
            current_texture_rid: None,
        })
    }

    pub fn import_and_copy(
        &mut self,
        info: &cef::AcceleratedPaintInfo,
        dst_rd_rid: Rid,
    ) -> Result<u64, String> {
        self.d3d12_importer.check_device_state()?;

        let handle = HANDLE(info.shared_texture_handle);
        if handle.is_invalid() {
            return Err("Source handle is invalid".into());
        }

        let width = info.extra.coded_size.width as u32;
        let height = info.extra.coded_size.height as u32;

        if width == 0 || height == 0 {
            return Err(format!("Invalid source dimensions: {}x{}", width, height));
        }
        if !dst_rd_rid.is_valid() {
            return Err("Destination RID is invalid".into());
        }

        let src_resource = self.d3d12_importer.import_shared_handle(
            handle,
            width,
            height,
            *info.format.as_ref(),
        )?;

        // Get destination D3D12 resource from Godot's RenderingDevice
        let dst_resource = {
            let mut rd = RenderingServer::singleton()
                .get_rendering_device()
                .ok_or("Failed to get RenderingDevice")?;

            let resource_ptr = rd.get_driver_resource(DriverResource::TEXTURE, dst_rd_rid, 0);

            if resource_ptr == 0 {
                return Err("Failed to get destination D3D12 resource handle".into());
            }

            unsafe { ID3D12Resource::from_raw(resource_ptr as *mut c_void) }
        };

        // Must wait for copy - D3D12 command lists don't AddRef resources
        let copy_id = self
            .d3d12_importer
            .queue_copy_texture(&src_resource, &dst_resource)?;

        self.d3d12_importer.wait_for_copy(copy_id)?;
        std::mem::forget(dst_resource);
        Ok(copy_id)
    }

    pub fn is_copy_complete(&self, copy_id: u64) -> bool {
        self.d3d12_importer.is_copy_complete(copy_id)
    }

    pub fn wait_for_all_copies(&self) {
        self.d3d12_importer.wait_for_all_copies()
    }
}

impl Drop for NativeTextureImporter {
    fn drop(&mut self) {
        if !self.fence_event.is_invalid() {
            let _ = unsafe { CloseHandle(self.fence_event) };
        }
    }
}

impl Drop for GodotTextureImporter {
    fn drop(&mut self) {
        if let Some(rid) = self.current_texture_rid.take() {
            RenderingServer::singleton().free_rid(rid);
        }
    }
}

pub fn is_supported() -> bool {
    NativeTextureImporter::new().is_some() && RenderBackend::detect().supports_accelerated_osr()
}

unsafe impl Send for GodotTextureImporter {}
unsafe impl Sync for GodotTextureImporter {}
