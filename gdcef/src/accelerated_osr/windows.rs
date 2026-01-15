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

/// D3D12 device and resources for importing shared textures from CEF.
/// Uses Godot's D3D12 device obtained via RenderingDevice::get_driver_resource()
/// to ensure resource compatibility for GPU copy operations.
pub struct NativeTextureImporter {
    /// D3D12 device borrowed from Godot - wrapped in ManuallyDrop to prevent Release
    device: std::mem::ManuallyDrop<ID3D12Device>,
    /// Command queue - OWNED by us (not Godot's) to avoid synchronization conflicts
    command_queue: ID3D12CommandQueue,
    /// Command allocator - owned by us
    command_allocator: ID3D12CommandAllocator,
    /// Fence for synchronization - owned by us
    fence: ID3D12Fence,
    fence_value: u64,
    fence_event: HANDLE,
    /// Tracks in-flight copy operations: fence_value -> when signaled
    pending_copies: std::collections::HashMap<u64, u64>,
    /// Tracks if we've logged a device removed error to avoid spam
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

    /// Checks if the D3D12 device is in a valid state.
    /// Returns Ok(()) if device is healthy, Err with reason if removed/suspended.
    pub fn check_device_state(&mut self) -> Result<(), String> {
        let reason = unsafe { self.device.GetDeviceRemovedReason() };
        if reason.is_ok() {
            // Device is healthy, reset the logged flag
            self.device_removed_logged = false;
            Ok(())
        } else {
            // Device has been removed
            let msg = format!(
                "D3D12 device removed: {:?}",
                reason.err()
            );
            if !self.device_removed_logged {
                godot_warn!("[AcceleratedOSR/Windows] {}", msg);
                self.device_removed_logged = true;
            }
            Err("D3D12 device removed".into())
        }
    }

    /// Import a shared texture handle from CEF into a D3D12 resource.
    ///
    /// CEF shares textures via NT handles (created with D3D12_RESOURCE_FLAG_ALLOW_CROSS_ADAPTER
    /// or similar sharing flags). We open this handle to get access to the texture.
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
            // Check if this is a device removed error
            let device_reason = unsafe { self.device.GetDeviceRemovedReason() };
            if device_reason.is_err() {
                // Device is actually removed/suspended
                if !self.device_removed_logged {
                    godot_warn!(
                        "[AcceleratedOSR/Windows] Device suspended/removed: {:?}. \
                         This may happen when window is minimized or on multi-GPU systems.",
                        device_reason.err()
                    );
                    self.device_removed_logged = true;
                }
                return Err("D3D12 device removed".into());
            }

            // Device is healthy but OpenSharedHandle still failed
            // This often happens on multi-GPU systems where CEF uses a different adapter
            if !self.device_removed_logged {
                godot_warn!(
                    "[AcceleratedOSR/Windows] OpenSharedHandle failed: {:?}. \
                     This may indicate a multi-GPU configuration where CEF and Godot use different adapters.",
                    e
                );
                self.device_removed_logged = true;
            }
            return Err("D3D12 device removed".into());
        }

        // Reset the logged flag on success
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

    /// Copies from a source D3D12 resource to a destination D3D12 resource asynchronously.
    /// Returns immediately without waiting for GPU completion.
    /// Returns the fence value to check completion later.
    pub fn queue_copy_texture(
        &mut self,
        src_resource: &ID3D12Resource,
        dst_resource: &ID3D12Resource,
    ) -> Result<u64, String> {
        // CRITICAL: Wait for any previous copy to complete before reusing the command allocator.
        // D3D12 requires the allocator to be idle before Reset() can be called.
        if self.fence_value > 0 {
            let completed = unsafe { self.fence.GetCompletedValue() };
            if completed < self.fence_value {
                // Previous commands still running, wait for them
                unsafe {
                    self.fence
                        .SetEventOnCompletion(self.fence_value, self.fence_event)
                }
                .map_err(|e| format!("Failed to set event on completion: {:?}", e))?;
                unsafe { WaitForSingleObject(self.fence_event, INFINITE) };
            }
        }

        // Now safe to reset the command allocator
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

        // For cross-process shared resources from CEF:
        // - The source resource is in COMMON state (implicit for shared resources)
        // - We should NOT transition the source resource - CEF owns it
        // - We only transition the destination resource which we own

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

        unsafe { command_list.ResourceBarrier(&[dst_barrier]) };

        // Copy the resource
        // Note: CopyResource can read from COMMON state resources (shared textures)
        unsafe { command_list.CopyResource(dst_resource, src_resource) };

        // Transition destination back to COMMON state for shader read
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

        // Signal fence WITHOUT waiting
        self.fence_value += 1;
        unsafe { self.command_queue.Signal(&self.fence, self.fence_value) }
            .map_err(|e| format!("Failed to signal fence: {:?}", e))?;

        let copy_id = self.fence_value;
        self.pending_copies.insert(copy_id, self.fence_value);

        Ok(copy_id)
    }

    /// Checks if an async copy has completed
    pub fn is_copy_complete(&self, copy_id: u64) -> bool {
        let completed_value = unsafe { self.fence.GetCompletedValue() };
        copy_id <= completed_value
    }

    /// Waits for all pending GPU copies to complete.
    /// MUST be called before freeing any destination textures.
    pub fn wait_for_all_copies(&self) {
        if self.fence_value == 0 {
            return;
        }

        let completed = unsafe { self.fence.GetCompletedValue() };
        if completed < self.fence_value {
            // Set up event and wait
            let result = unsafe {
                self.fence.SetEventOnCompletion(self.fence_value, self.fence_event)
            };
            if result.is_ok() {
                unsafe { WaitForSingleObject(self.fence_event, INFINITE) };
            }
        }
    }

    /// Waits for a specific copy operation to complete.
    /// MUST be called before releasing the source resource.
    pub fn wait_for_copy(&self, copy_id: u64) -> Result<(), String> {
        let completed = unsafe { self.fence.GetCompletedValue() };
        if completed >= copy_id {
            return Ok(());
        }

        // Set up event and wait for this specific copy
        unsafe {
            self.fence.SetEventOnCompletion(copy_id, self.fence_event)
        }
        .map_err(|e| format!("Failed to set event on completion: {:?}", e))?;

        unsafe { WaitForSingleObject(self.fence_event, INFINITE) };
        Ok(())
    }
}

/// Imports D3D12 shared textures from CEF into Godot's rendering system.
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

    /// Imports a CEF shared texture and immediately performs a GPU copy.
    /// This should be called during on_accelerated_paint while the handle is guaranteed valid.
    ///
    /// # Arguments
    /// * `info` - The accelerated paint info from CEF containing the shared texture handle
    /// * `dst_rd_rid` - The RenderingDevice RID of the destination Godot texture
    ///
    /// # Returns
    /// * `Ok(copy_id)` - Copy completed successfully
    /// * `Err(String)` - Error description on failure
    pub fn import_and_copy(
        &mut self,
        info: &cef::AcceleratedPaintInfo,
        dst_rd_rid: Rid,
    ) -> Result<u64, String> {
        // Check device state first - skip gracefully if device is suspended/removed
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

        // Import the shared handle into a D3D12 resource immediately while valid
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

        // Queue the copy and wait for it to complete synchronously.
        // CRITICAL: We MUST wait because D3D12 command lists do NOT AddRef resources.
        // If we return before the copy completes, src_resource will be released while
        // the GPU is still reading from it, causing use-after-free and eventual TDR.
        let copy_id = self
            .d3d12_importer
            .queue_copy_texture(&src_resource, &dst_resource)?;

        // Wait for the copy to complete before releasing src_resource
        // This ensures the GPU is done reading from the source texture
        self.d3d12_importer.wait_for_copy(copy_id)?;

        // Don't forget the dst_resource - it's borrowed from Godot
        std::mem::forget(dst_resource);

        // src_resource will be dropped here, but that's safe now because
        // the GPU copy has completed.

        Ok(copy_id)
    }

    /// Checks if an async copy operation has completed.
    /// Returns true if completed, false if still in progress.
    pub fn is_copy_complete(&self, copy_id: u64) -> bool {
        self.d3d12_importer.is_copy_complete(copy_id)
    }

    /// Waits for all pending GPU copies to complete.
    /// MUST be called before freeing any destination textures to avoid use-after-free.
    pub fn wait_for_all_copies(&self) {
        self.d3d12_importer.wait_for_all_copies()
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
    }
}

pub fn is_supported() -> bool {
    NativeTextureImporter::new().is_some() && RenderBackend::detect().supports_accelerated_osr()
}
