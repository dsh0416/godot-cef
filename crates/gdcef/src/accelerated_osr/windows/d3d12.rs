use godot::classes::RenderingServer;
use godot::classes::rendering_device::DriverResource;
use godot::global::{godot_error, godot_print, godot_warn};
use godot::prelude::*;
use std::ffi::c_void;
use windows::Win32::Foundation::{
    CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE, LUID,
};
use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_BGRA_SUPPORT, ID3D11Device, ID3D11Device1,
    ID3D11DeviceContext, ID3D11Resource, ID3D11Texture2D,
};
use windows::Win32::Graphics::Direct3D11on12::{
    D3D11_RESOURCE_FLAGS, D3D11On12CreateDevice, ID3D11On12Device,
};
use windows::Win32::Graphics::Direct3D12::{
    D3D12_COMMAND_LIST_TYPE_DIRECT, D3D12_COMMAND_QUEUE_DESC, D3D12_RESOURCE_BARRIER,
    D3D12_RESOURCE_BARRIER_0, D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
    D3D12_RESOURCE_BARRIER_FLAG_NONE, D3D12_RESOURCE_BARRIER_TYPE_TRANSITION, D3D12_RESOURCE_DESC,
    D3D12_RESOURCE_DIMENSION_TEXTURE2D, D3D12_RESOURCE_STATE_COMMON,
    D3D12_RESOURCE_STATE_COPY_DEST, D3D12_RESOURCE_TRANSITION_BARRIER, ID3D12CommandAllocator,
    ID3D12CommandQueue, ID3D12Device, ID3D12Fence, ID3D12GraphicsCommandList, ID3D12Resource,
};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory, IDXGIAdapter, IDXGIFactory};
use windows::Win32::System::Threading::{
    CreateEventW, GetCurrentProcess, INFINITE, WaitForSingleObject,
};
use windows::core::{IUnknown, Interface};

pub struct PendingD3D12Copy {
    duplicated_handle: HANDLE,
    width: u32,
    height: u32,
}

impl Drop for PendingD3D12Copy {
    fn drop(&mut self) {
        if !self.duplicated_handle.is_invalid() {
            let _ = unsafe { CloseHandle(self.duplicated_handle) };
        }
    }
}

struct ImportedD3D12Resource {
    duplicated_handle: HANDLE,
    #[allow(dead_code)]
    resource: ID3D12Resource,
}

fn duplicate_win32_handle(handle: HANDLE) -> Result<HANDLE, String> {
    let mut duplicated = HANDLE::default();
    let current_process = unsafe { GetCurrentProcess() };
    unsafe {
        DuplicateHandle(
            current_process,
            handle,
            current_process,
            &mut duplicated,
            0,
            false,
            DUPLICATE_SAME_ACCESS,
        )
        .map_err(|e| format!("DuplicateHandle failed: {:?}", e))?;
    }
    Ok(duplicated)
}

struct D3D11On12Fallback {
    d3d11_device: ID3D11Device,
    d3d11_context: ID3D11DeviceContext,
    d3d11on12_device: ID3D11On12Device,
}

impl D3D11On12Fallback {
    fn new(device: &ID3D12Device, command_queue: &ID3D12CommandQueue) -> Option<Self> {
        let command_queue_unknown: IUnknown = match command_queue.cast() {
            Ok(queue) => queue,
            Err(e) => {
                godot_warn!(
                    "[AcceleratedOSR/D3D12] Failed to cast command queue to IUnknown: {:?}",
                    e
                );
                return None;
            }
        };

        let command_queues = [Some(command_queue_unknown)];
        let feature_levels = [D3D_FEATURE_LEVEL_11_0];
        let mut d3d11_device: Option<ID3D11Device> = None;
        let mut d3d11_context: Option<ID3D11DeviceContext> = None;

        if let Err(e) = unsafe {
            D3D11On12CreateDevice(
                device,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT.0,
                Some(&feature_levels),
                Some(&command_queues),
                0,
                Some(&mut d3d11_device),
                Some(&mut d3d11_context),
                None,
            )
        } {
            godot_warn!(
                "[AcceleratedOSR/D3D12] Failed to create D3D11On12 device: {:?}",
                e
            );
            return None;
        }

        let Some(d3d11_device) = d3d11_device else {
            godot_warn!("[AcceleratedOSR/D3D12] D3D11On12 device creation returned null device");
            return None;
        };
        let Some(d3d11_context) = d3d11_context else {
            godot_warn!("[AcceleratedOSR/D3D12] D3D11On12 device creation returned null context");
            return None;
        };

        let d3d11on12_device = match d3d11_device.cast::<ID3D11On12Device>() {
            Ok(device) => device,
            Err(e) => {
                godot_warn!(
                    "[AcceleratedOSR/D3D12] Failed to query ID3D11On12Device: {:?}",
                    e
                );
                return None;
            }
        };

        Some(Self {
            d3d11_device,
            d3d11_context,
            d3d11on12_device,
        })
    }

    fn copy_from_handle(
        &self,
        handle: HANDLE,
        dst_resource: &ID3D12Resource,
        command_queue: &ID3D12CommandQueue,
        fence: &ID3D12Fence,
        fence_value: &mut u64,
    ) -> Result<(), String> {
        let d3d11_device1: ID3D11Device1 = self
            .d3d11_device
            .cast()
            .map_err(|e| format!("Failed to query ID3D11Device1: {:?}", e))?;
        let src_texture: ID3D11Texture2D = unsafe { d3d11_device1.OpenSharedResource1(handle) }
            .map_err(|e| format!("D3D11 OpenSharedResource1 failed: {:?}", e))?;
        let src_resource: ID3D11Resource = src_texture
            .cast()
            .map_err(|e| format!("Failed to cast source texture: {:?}", e))?;

        let flags = D3D11_RESOURCE_FLAGS {
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            ..Default::default()
        };
        let mut wrapped_resource: Option<ID3D11Resource> = None;
        unsafe {
            self.d3d11on12_device.CreateWrappedResource(
                dst_resource,
                &flags,
                D3D12_RESOURCE_STATE_COMMON,
                D3D12_RESOURCE_STATE_COMMON,
                &mut wrapped_resource,
            )
        }
        .map_err(|e| format!("CreateWrappedResource failed: {:?}", e))?;

        let wrapped_resource = wrapped_resource
            .ok_or_else(|| "CreateWrappedResource returned null resource".to_string())?;
        let wrapped_resources = [Some(wrapped_resource.clone())];

        unsafe {
            self.d3d11on12_device
                .AcquireWrappedResources(&wrapped_resources)
        };
        unsafe {
            self.d3d11_context
                .CopyResource(&wrapped_resource, &src_resource)
        };
        unsafe {
            self.d3d11on12_device
                .ReleaseWrappedResources(&wrapped_resources)
        };
        unsafe { self.d3d11_context.Flush() };

        *fence_value += 1;
        unsafe { command_queue.Signal(fence, *fence_value) }
            .map_err(|e| format!("Failed to signal fence (D3D11 path): {:?}", e))?;

        Ok(())
    }
}

pub struct D3D12TextureImporter {
    device: std::mem::ManuallyDrop<ID3D12Device>,
    command_queue: ID3D12CommandQueue,
    command_allocator: ID3D12CommandAllocator,
    fence: ID3D12Fence,
    fence_value: u64,
    fence_event: HANDLE,
    device_removed_logged: bool,
    pending_copy: Option<PendingD3D12Copy>,
    imported_resource: Option<ImportedD3D12Resource>,
    copy_in_flight: bool,
    d3d11_fallback: Option<D3D11On12Fallback>,
}

impl D3D12TextureImporter {
    pub fn new() -> Option<Self> {
        let mut rd = RenderingServer::singleton()
            .get_rendering_device()
            .ok_or_else(|| {
                godot_error!("[AcceleratedOSR/D3D12] Failed to get RenderingDevice");
            })
            .ok()?;

        let device_ptr = rd.get_driver_resource(DriverResource::LOGICAL_DEVICE, Rid::Invalid, 0);

        if device_ptr == 0 {
            godot_error!("[AcceleratedOSR/D3D12] Failed to get D3D12 device from Godot");
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
                    "[AcceleratedOSR/D3D12] Failed to create command queue: {:?}",
                    e
                )
            })
            .ok()?;

        let d3d11_fallback = D3D11On12Fallback::new(&device, &command_queue);

        // Create command allocator using Godot's device
        let command_allocator: ID3D12CommandAllocator =
            unsafe { device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT) }
                .map_err(|e| {
                    godot_error!(
                        "[AcceleratedOSR/D3D12] Failed to create command allocator: {:?}",
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
        .map_err(|e| godot_error!("[AcceleratedOSR/D3D12] Failed to create fence: {:?}", e))
        .ok()?;

        let fence_event = unsafe { CreateEventW(None, false, false, None) }
            .map_err(|e| {
                godot_error!(
                    "[AcceleratedOSR/D3D12] Failed to create fence event: {:?}",
                    e
                )
            })
            .ok()?;

        godot_print!("[AcceleratedOSR/D3D12] Using Godot's D3D12 device for accelerated OSR");

        Some(Self {
            device: std::mem::ManuallyDrop::new(device),
            command_queue,
            command_allocator,
            fence,
            fence_value: 0,
            fence_event,
            device_removed_logged: false,
            pending_copy: None,
            imported_resource: None,
            copy_in_flight: false,
            d3d11_fallback,
        })
    }

    pub fn check_device_state(&mut self) -> Result<(), String> {
        let reason = unsafe { self.device.GetDeviceRemovedReason() };
        if reason.is_ok() {
            self.device_removed_logged = false;
            Ok(())
        } else if !self.device_removed_logged {
            godot_warn!(
                "[AcceleratedOSR/D3D12] D3D12 device removed: {:?}",
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
            if device_reason.is_err() {
                if !self.device_removed_logged {
                    godot_warn!(
                        "[AcceleratedOSR/D3D12] Device removed: {:?}",
                        device_reason.err()
                    );
                    self.device_removed_logged = true;
                }
                return Err("D3D12 device removed".into());
            }

            // Handle is not valid for D3D12. This can happen when CEF produces a D3D11
            // shared texture handle (opaque NT handle with no API tag).
            godot_warn!(
                "[AcceleratedOSR/D3D12] OpenSharedHandle failed (likely non-D3D12 handle): {:?}",
                e
            );
            return Err("Shared handle is not a D3D12 resource".into());
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

    pub fn queue_copy(&mut self, info: &cef::AcceleratedPaintInfo) -> Result<(), String> {
        let handle = HANDLE(info.shared_texture_handle);
        if handle.is_invalid() {
            return Err("Source handle is invalid".into());
        }

        let width = info.extra.coded_size.width as u32;
        let height = info.extra.coded_size.height as u32;

        if width == 0 || height == 0 {
            return Err(format!("Invalid source dimensions: {}x{}", width, height));
        }

        // Duplicate the handle so we own it - this is fast and non-blocking
        let duplicated_handle = duplicate_win32_handle(handle)?;

        // Replace any existing pending copy (drop the old one, which closes its handle)
        self.pending_copy = Some(PendingD3D12Copy {
            duplicated_handle,
            width,
            height,
        });

        Ok(())
    }

    pub fn process_pending_copy(&mut self, dst_rd_rid: Rid) -> Result<(), String> {
        self.check_device_state()?;

        let pending = match self.pending_copy.take() {
            Some(p) => p,
            None => return Ok(()), // Nothing to do
        };

        if !dst_rd_rid.is_valid() {
            return Err("Destination RID is invalid".into());
        }

        // Wait for any previous in-flight copy to complete before reusing resources
        if self.copy_in_flight {
            self.wait_for_copy()?;
            self.copy_in_flight = false;
        }

        // Free previous imported resource
        self.free_imported_resource();

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

        // Import the resource using our duplicated handle
        let src_resource = match self.import_shared_handle(
            pending.duplicated_handle,
            pending.width,
            pending.height,
            cef::sys::cef_color_type_t::CEF_COLOR_TYPE_BGRA_8888,
        ) {
            Ok(res) => Some(res),
            Err(e) => {
                if e.contains("Shared handle is not a D3D12 resource") {
                    None
                } else {
                    // pending will be dropped here, closing its handle
                    return Err(e);
                }
            }
        };

        if let Some(src_resource) = src_resource {
            // Submit copy command (non-blocking)
            self.submit_copy_async(&src_resource, &dst_resource)?;
            self.copy_in_flight = true;

            // Don't drop dst_resource - it's owned by Godot
            std::mem::forget(dst_resource);

            // Store the imported resource (keeps it alive for the GPU operation)
            // Transfer handle ownership from pending to imported_resource
            self.imported_resource = Some(ImportedD3D12Resource {
                duplicated_handle: pending.duplicated_handle,
                resource: src_resource,
            });

            // Prevent pending's Drop from closing the handle (we transferred ownership)
            std::mem::forget(pending);
        } else {
            godot_warn!(
                "[AcceleratedOSR/D3D12] D3D12 open failed; trying D3D11 handle via D3D11On12"
            );
            let Some(d3d11_fallback) = &self.d3d11_fallback else {
                return Err("D3D11 fallback is not initialized".into());
            };
            d3d11_fallback.copy_from_handle(
                pending.duplicated_handle,
                &dst_resource,
                &self.command_queue,
                &self.fence,
                &mut self.fence_value,
            )?;
            self.copy_in_flight = true;

            // Don't drop dst_resource - it's owned by Godot
            std::mem::forget(dst_resource);
            // pending will be dropped here, closing its handle
        }

        Ok(())
    }

    pub fn wait_for_copy(&mut self) -> Result<(), String> {
        if !self.copy_in_flight {
            return Ok(());
        }

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

        self.copy_in_flight = false;
        Ok(())
    }

    fn submit_copy_async(
        &mut self,
        src_resource: &ID3D12Resource,
        dst_resource: &ID3D12Resource,
    ) -> Result<(), String> {
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

        // NOTE: We do NOT wait here - the caller should call wait_for_copy() when needed
        Ok(())
    }

    fn free_imported_resource(&mut self) {
        if let Some(imported) = self.imported_resource.take() {
            let _ = unsafe { CloseHandle(imported.duplicated_handle) };
        }
    }
}

impl Drop for D3D12TextureImporter {
    fn drop(&mut self) {
        if self.copy_in_flight {
            let _ = self.wait_for_copy();
        }

        self.pending_copy = None;
        self.free_imported_resource();

        if !self.fence_event.is_invalid() {
            let _ = unsafe { CloseHandle(self.fence_event) };
        }
    }
}

/// Get the GPU vendor and device IDs from Godot's D3D12 device.
pub fn get_godot_gpu_device_ids() -> Option<(u32, u32)> {
    let mut rd = RenderingServer::singleton().get_rendering_device()?;
    let device_ptr = rd.get_driver_resource(DriverResource::LOGICAL_DEVICE, Rid::Invalid, 0);

    if device_ptr == 0 {
        godot_warn!("[AcceleratedOSR/D3D12] Failed to get D3D12 device for GPU ID query");
        return None;
    }

    let device: ID3D12Device = unsafe { ID3D12Device::from_raw(device_ptr as *mut c_void) };
    let target_luid: LUID = unsafe { device.GetAdapterLuid() };

    // Device is from Godot, we don't need to close it
    std::mem::forget(device);

    // Use the original DXGI factory (available since Windows Vista) for maximum compatibility
    let factory: IDXGIFactory = unsafe { CreateDXGIFactory() }.ok()?;

    let mut adapter_index = 0u32;
    loop {
        let adapter: IDXGIAdapter = match unsafe { factory.EnumAdapters(adapter_index) } {
            Ok(a) => a,
            Err(_) => break, // No more adapters
        };

        let desc = match unsafe { adapter.GetDesc() } {
            Ok(d) => d,
            Err(_) => {
                adapter_index += 1;
                continue;
            }
        };

        if desc.AdapterLuid.HighPart == target_luid.HighPart
            && desc.AdapterLuid.LowPart == target_luid.LowPart
        {
            let name = String::from_utf16_lossy(&desc.Description)
                .trim_end_matches('\0')
                .to_string();
            godot_print!(
                "[AcceleratedOSR/D3D12] Godot GPU: vendor=0x{:04x}, device=0x{:04x}, name={}",
                desc.VendorId,
                desc.DeviceId,
                name
            );
            return Some((desc.VendorId, desc.DeviceId));
        }

        adapter_index += 1;
    }

    godot_warn!("[AcceleratedOSR/D3D12] Could not find adapter matching LUID");
    None
}

unsafe impl Send for D3D12TextureImporter {}
unsafe impl Sync for D3D12TextureImporter {}
