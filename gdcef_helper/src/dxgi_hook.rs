//! DXGI Adapter Filtering via Detours
//!
//! This module hooks DXGI factory creation functions to filter GPU adapters,
//! ensuring CEF uses the same adapter as Godot for DX12 shared texture compatibility.
//!
//! The approach:
//! 1. Hook CreateDXGIFactory1/2 to intercept factory creation
//! 2. After the real factory is created, patch its vtable to redirect adapter enumeration methods:
//!    - EnumAdapters (IDXGIFactory)
//!    - EnumAdapters1 (IDXGIFactory1)
//!    - EnumAdapterByLuid (IDXGIFactory4) - for direct LUID lookups
//!    - EnumAdapterByGpuPreference (IDXGIFactory6) - for GPU preference selection
//! 3. Our hooked functions hide all adapters except the target - only index 0 is valid

use std::ffi::c_void;
use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

use retour::static_detour;
use windows::Win32::Foundation::LUID;
use windows::Win32::Graphics::Dxgi::{
    IDXGIAdapter1, IDXGIFactory1, IDXGIFactory2, IDXGIFactory4, IDXGIFactory6,
};
use windows::Win32::System::Memory::{
    PAGE_EXECUTE_READWRITE, PAGE_PROTECTION_FLAGS, VirtualProtect,
};
use windows::core::{GUID, HRESULT, IUnknown, Interface};

static TARGET_LUID: OnceLock<LUID> = OnceLock::new();
static HOOKS_INSTALLED: OnceLock<bool> = OnceLock::new();
static TARGET_ADAPTER_INDEX: AtomicU32 = AtomicU32::new(u32::MAX);
static ORIGINAL_ENUM_ADAPTERS: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static ORIGINAL_ENUM_ADAPTERS1: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static ORIGINAL_ENUM_ADAPTER_BY_LUID: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static ORIGINAL_ENUM_ADAPTER_BY_GPU_PREFERENCE: AtomicPtr<c_void> =
    AtomicPtr::new(std::ptr::null_mut());
static VTABLE_PATCH_LOCK: Mutex<()> = Mutex::new(());

// Raw function signatures for hooking (these match the actual DLL exports)
type CreateDXGIFactory1Fn = unsafe extern "system" fn(*const GUID, *mut *mut c_void) -> HRESULT;
type CreateDXGIFactory2Fn =
    unsafe extern "system" fn(u32, *const GUID, *mut *mut c_void) -> HRESULT;

// EnumAdapters/EnumAdapters1 method signature (COM calling convention)
// Both have the same ABI signature: (this, adapter_index, pp_adapter) -> HRESULT
type EnumAdaptersFn = unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT;

// EnumAdapterByLuid (IDXGIFactory4) signature
// (this, adapter_luid, riid, pp_adapter) -> HRESULT
type EnumAdapterByLuidFn =
    unsafe extern "system" fn(*mut c_void, LUID, *const GUID, *mut *mut c_void) -> HRESULT;

// EnumAdapterByGpuPreference (IDXGIFactory6) signature
// (this, adapter_index, gpu_preference, riid, pp_adapter) -> HRESULT
// DXGI_GPU_PREFERENCE is an enum: 0 = UNSPECIFIED, 1 = MINIMUM_POWER, 2 = HIGH_PERFORMANCE
type EnumAdapterByGpuPreferenceFn =
    unsafe extern "system" fn(*mut c_void, u32, i32, *const GUID, *mut *mut c_void) -> HRESULT;

static_detour! {
    static CreateDXGIFactory1Hook: unsafe extern "system" fn(*const GUID, *mut *mut c_void) -> HRESULT;
    static CreateDXGIFactory2Hook: unsafe extern "system" fn(u32, *const GUID, *mut *mut c_void) -> HRESULT;
}

pub fn set_target_luid(luid: LUID) {
    TARGET_LUID.set(luid).ok();
}

pub fn get_target_luid() -> Option<&'static LUID> {
    TARGET_LUID.get()
}

/// Find the target adapter index using the ORIGINAL (unhooked) EnumAdapters1 if available.
/// This is important because the vtable might already be patched from a previous factory creation.
fn find_target_adapter_index(factory: &IDXGIFactory1, target: &LUID) -> Option<u32> {
    let mut index = 0u32;

    // Check if we have the original function pointer (vtable might be patched)
    let original_ptr = ORIGINAL_ENUM_ADAPTERS1.load(Ordering::SeqCst);

    loop {
        let adapter_result = if !original_ptr.is_null() {
            // Use the original function directly to bypass our hook
            unsafe {
                let original: EnumAdaptersFn = std::mem::transmute(original_ptr);
                // Get the raw COM interface pointer, not the Rust wrapper address
                let factory_ptr = factory.as_raw();
                let mut adapter_ptr: *mut c_void = std::ptr::null_mut();
                let hr = original(factory_ptr, index, &mut adapter_ptr);
                if hr.is_ok() && !adapter_ptr.is_null() {
                    Ok(IDXGIAdapter1::from_raw(adapter_ptr))
                } else {
                    Err(())
                }
            }
        } else {
            // No original stored yet, use the vtable (should be unpatched on first call)
            unsafe { factory.EnumAdapters1(index) }.map_err(|_| ())
        };

        let Ok(adapter) = adapter_result else {
            break;
        };

        if let Ok(desc) = unsafe { adapter.GetDesc1() } {
            let name = String::from_utf16_lossy(&desc.Description)
                .trim_end_matches('\0')
                .to_string();
            eprintln!(
                "[DXGI Hook] Adapter {}: LUID ({}, {}), Name: {}",
                index, desc.AdapterLuid.HighPart, desc.AdapterLuid.LowPart, name
            );

            if desc.AdapterLuid.HighPart == target.HighPart
                && desc.AdapterLuid.LowPart == target.LowPart
            {
                return Some(index);
            }
        }

        if index == u32::MAX {
            break;
        }
        index += 1;
    }
    None
}

// DXGI_ERROR_NOT_FOUND
const DXGI_ERROR_NOT_FOUND: HRESULT = HRESULT(0x887A0002_u32 as i32);

/// Hooked EnumAdapters (IDXGIFactory) - only returns target adapter at index 0.
unsafe extern "system" fn hooked_enum_adapters(
    this: *mut c_void,
    adapter_index: u32,
    pp_adapter: *mut *mut c_void,
) -> HRESULT {
    let target_index = TARGET_ADAPTER_INDEX.load(Ordering::SeqCst);

    // If we haven't found a target adapter, pass through unchanged
    if target_index == u32::MAX {
        unsafe {
            let original_ptr = ORIGINAL_ENUM_ADAPTERS.load(Ordering::SeqCst);
            if original_ptr.is_null() {
                return HRESULT::from_win32(windows::Win32::Foundation::ERROR_INVALID_FUNCTION.0);
            }
            let original: EnumAdaptersFn = std::mem::transmute(original_ptr);
            return original(this, adapter_index, pp_adapter);
        }
    }

    // Only index 0 is valid - it returns the target adapter
    // All other indices return NOT_FOUND to hide other adapters
    if adapter_index != 0 {
        return DXGI_ERROR_NOT_FOUND;
    }

    unsafe {
        let original_ptr = ORIGINAL_ENUM_ADAPTERS.load(Ordering::SeqCst);
        if original_ptr.is_null() {
            return HRESULT::from_win32(windows::Win32::Foundation::ERROR_INVALID_FUNCTION.0);
        }
        let original: EnumAdaptersFn = std::mem::transmute(original_ptr);
        original(this, target_index, pp_adapter)
    }
}

/// Hooked EnumAdapters1 (IDXGIFactory1) - only returns target adapter at index 0.
unsafe extern "system" fn hooked_enum_adapters1(
    this: *mut c_void,
    adapter_index: u32,
    pp_adapter: *mut *mut c_void,
) -> HRESULT {
    let target_index = TARGET_ADAPTER_INDEX.load(Ordering::SeqCst);

    // If we haven't found a target adapter, pass through unchanged
    if target_index == u32::MAX {
        unsafe {
            let original_ptr = ORIGINAL_ENUM_ADAPTERS1.load(Ordering::SeqCst);
            if original_ptr.is_null() {
                return HRESULT::from_win32(windows::Win32::Foundation::ERROR_INVALID_FUNCTION.0);
            }
            let original: EnumAdaptersFn = std::mem::transmute(original_ptr);
            return original(this, adapter_index, pp_adapter);
        }
    }

    // Only index 0 is valid - it returns the target adapter
    // All other indices return NOT_FOUND to hide other adapters
    if adapter_index != 0 {
        return DXGI_ERROR_NOT_FOUND;
    }

    unsafe {
        let original_ptr = ORIGINAL_ENUM_ADAPTERS1.load(Ordering::SeqCst);
        if original_ptr.is_null() {
            return HRESULT::from_win32(windows::Win32::Foundation::ERROR_INVALID_FUNCTION.0);
        }
        let original: EnumAdaptersFn = std::mem::transmute(original_ptr);
        original(this, target_index, pp_adapter)
    }
}

/// Hooked EnumAdapterByLuid (IDXGIFactory4) - only allows our target adapter's LUID.
unsafe extern "system" fn hooked_enum_adapter_by_luid(
    this: *mut c_void,
    adapter_luid: LUID,
    riid: *const GUID,
    pp_adapter: *mut *mut c_void,
) -> HRESULT {
    let target_index = TARGET_ADAPTER_INDEX.load(Ordering::SeqCst);

    // If we haven't found a target adapter, pass through unchanged
    if target_index == u32::MAX {
        unsafe {
            let original_ptr = ORIGINAL_ENUM_ADAPTER_BY_LUID.load(Ordering::SeqCst);
            if original_ptr.is_null() {
                return HRESULT::from_win32(windows::Win32::Foundation::ERROR_INVALID_FUNCTION.0);
            }
            let original: EnumAdapterByLuidFn = std::mem::transmute(original_ptr);
            return original(this, adapter_luid, riid, pp_adapter);
        }
    }

    // Check if the requested LUID matches our target
    if let Some(target_luid) = get_target_luid()
        && adapter_luid.HighPart == target_luid.HighPart
        && adapter_luid.LowPart == target_luid.LowPart
    {
        unsafe {
            let original_ptr = ORIGINAL_ENUM_ADAPTER_BY_LUID.load(Ordering::SeqCst);
            if original_ptr.is_null() {
                return HRESULT::from_win32(windows::Win32::Foundation::ERROR_INVALID_FUNCTION.0);
            }
            let original: EnumAdapterByLuidFn = std::mem::transmute(original_ptr);
            return original(this, adapter_luid, riid, pp_adapter);
        }
    }

    eprintln!(
        "[DXGI Hook] EnumAdapterByLuid blocked for LUID ({}, {})",
        adapter_luid.HighPart, adapter_luid.LowPart
    );
    DXGI_ERROR_NOT_FOUND
}

/// Hooked EnumAdapterByGpuPreference (IDXGIFactory6) - always returns our target adapter at index 0.
unsafe extern "system" fn hooked_enum_adapter_by_gpu_preference(
    this: *mut c_void,
    adapter_index: u32,
    gpu_preference: i32,
    riid: *const GUID,
    pp_adapter: *mut *mut c_void,
) -> HRESULT {
    let target_index = TARGET_ADAPTER_INDEX.load(Ordering::SeqCst);

    // If we haven't found a target adapter, pass through unchanged
    if target_index == u32::MAX {
        unsafe {
            let original_ptr = ORIGINAL_ENUM_ADAPTER_BY_GPU_PREFERENCE.load(Ordering::SeqCst);
            if original_ptr.is_null() {
                return HRESULT::from_win32(windows::Win32::Foundation::ERROR_INVALID_FUNCTION.0);
            }
            let original: EnumAdapterByGpuPreferenceFn = std::mem::transmute(original_ptr);
            return original(this, adapter_index, gpu_preference, riid, pp_adapter);
        }
    }

    // Only index 0 is valid - it returns the target adapter
    // All other indices return NOT_FOUND to hide other adapters
    if adapter_index != 0 {
        return DXGI_ERROR_NOT_FOUND;
    }

    unsafe {
        // Always prefer using the original EnumAdapterByGpuPreference to honor the riid parameter.
        // Using EnumAdapters1 would return IDXGIAdapter1, ignoring the caller's requested interface.
        let original_pref_ptr = ORIGINAL_ENUM_ADAPTER_BY_GPU_PREFERENCE.load(Ordering::SeqCst);
        if !original_pref_ptr.is_null() {
            let original: EnumAdapterByGpuPreferenceFn = std::mem::transmute(original_pref_ptr);
            return original(this, target_index, gpu_preference, riid, pp_adapter);
        }

        // Fallback: use EnumAdapters1 and QueryInterface to the requested interface
        let original_enum1_ptr = ORIGINAL_ENUM_ADAPTERS1.load(Ordering::SeqCst);
        if original_enum1_ptr.is_null() {
            return HRESULT::from_win32(windows::Win32::Foundation::ERROR_INVALID_FUNCTION.0);
        }

        let original_enum1: EnumAdaptersFn = std::mem::transmute(original_enum1_ptr);
        let mut temp_adapter: *mut c_void = std::ptr::null_mut();
        let hr = original_enum1(this, target_index, &mut temp_adapter);
        if hr.is_err() || temp_adapter.is_null() {
            return hr;
        }

        let adapter_unknown = IUnknown::from_raw(temp_adapter);
        adapter_unknown.query(riid, pp_adapter)
    }
}

unsafe fn get_vtable(obj: *mut c_void) -> *mut *mut c_void {
    unsafe { *(obj as *mut *mut *mut c_void) }
}

// VTable indices for EnumAdapters methods:
// IUnknown: 3 methods (QueryInterface, AddRef, Release) - indices 0-2
// IDXGIObject: 4 methods (SetPrivateData, SetPrivateDataInterface, GetPrivateData, GetParent) - indices 3-6
// IDXGIFactory: 5 methods (EnumAdapters, MakeWindowAssociation, GetWindowAssociation, CreateSwapChain, CreateSoftwareAdapter) - indices 7-11
// IDXGIFactory1: 2 methods (EnumAdapters1, IsCurrent) - indices 12-13
// IDXGIFactory2: 11 methods (IsWindowedStereoEnabled, CreateSwapChainForHwnd, CreateSwapChainForCoreWindow,
//                           GetSharedResourceAdapterLuid, RegisterStereoStatusWindow, RegisterStereoStatusEvent,
//                           UnregisterStereoStatus, RegisterOcclusionStatusWindow, RegisterOcclusionStatusEvent,
//                           UnregisterOcclusionStatus, CreateSwapChainForComposition) - indices 14-24
// IDXGIFactory3: 1 method (GetCreationFlags) - index 25
// IDXGIFactory4: 2 methods (EnumAdapterByLuid, EnumWarpAdapter) - indices 26-27
// IDXGIFactory5: 1 method (CheckFeatureSupport) - index 28
// IDXGIFactory6: 2 methods (EnumAdapterByGpuPreference, RegisterAdaptersChangedEvent) - indices 29-30
const ENUM_ADAPTERS_VTABLE_INDEX: usize = 7;
const ENUM_ADAPTERS1_VTABLE_INDEX: usize = 12;
const ENUM_ADAPTER_BY_LUID_VTABLE_INDEX: usize = 26;
const ENUM_ADAPTER_BY_GPU_PREFERENCE_VTABLE_INDEX: usize = 29;

struct MemoryProtectionGuard {
    address: *const c_void,
    size: usize,
    old_protect: PAGE_PROTECTION_FLAGS,
    active: bool,
}

impl MemoryProtectionGuard {
    unsafe fn new(address: *const c_void, size: usize) -> Option<Self> {
        let mut old_protect = PAGE_PROTECTION_FLAGS(0);
        let result =
            unsafe { VirtualProtect(address, size, PAGE_EXECUTE_READWRITE, &mut old_protect) };

        if result.is_err() {
            return None;
        }

        Some(Self {
            address,
            size,
            old_protect,
            active: true,
        })
    }

    unsafe fn restore(&mut self) -> bool {
        if !self.active {
            return true;
        }

        let mut dummy = PAGE_PROTECTION_FLAGS(0);
        let result =
            unsafe { VirtualProtect(self.address, self.size, self.old_protect, &mut dummy) };

        self.active = false;
        result.is_ok()
    }
}

impl Drop for MemoryProtectionGuard {
    fn drop(&mut self) {
        if self.active {
            let mut dummy = PAGE_PROTECTION_FLAGS(0);
            let _ =
                unsafe { VirtualProtect(self.address, self.size, self.old_protect, &mut dummy) };
        }
    }
}

unsafe fn patch_vtable_slot(
    vtable: *mut *mut c_void,
    index: usize,
    original_storage: &AtomicPtr<c_void>,
    hook_fn: *mut c_void,
) -> bool {
    unsafe {
        let slot = vtable.add(index);
        let current = *slot;

        // Check if already patched
        if current == hook_fn {
            return true;
        }

        // Store original (only if not already set)
        let _ = original_storage.compare_exchange(
            std::ptr::null_mut(),
            current,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );

        let slot_ptr = slot as *const c_void;
        let slot_size = std::mem::size_of::<*mut c_void>();

        let Some(mut guard) = MemoryProtectionGuard::new(slot_ptr, slot_size) else {
            return false;
        };

        *slot = hook_fn;
        let _ = guard.restore();

        true
    }
}

unsafe fn patch_factory_vtable(factory_ptr: *mut c_void) -> bool {
    let _lock = match VTABLE_PATCH_LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    unsafe {
        if factory_ptr.is_null() {
            return false;
        }

        let vtable = get_vtable(factory_ptr);
        if vtable.is_null() {
            return false;
        }

        let enum_adapters_ok = patch_vtable_slot(
            vtable,
            ENUM_ADAPTERS_VTABLE_INDEX,
            &ORIGINAL_ENUM_ADAPTERS,
            hooked_enum_adapters as *mut c_void,
        );
        if !enum_adapters_ok {
            eprintln!("[DXGI Hook] Failed to patch EnumAdapters vtable slot");
        }

        // Patch EnumAdapters1 (IDXGIFactory1)
        let enum_adapters1_ok = patch_vtable_slot(
            vtable,
            ENUM_ADAPTERS1_VTABLE_INDEX,
            &ORIGINAL_ENUM_ADAPTERS1,
            hooked_enum_adapters1 as *mut c_void,
        );
        if !enum_adapters1_ok {
            eprintln!("[DXGI Hook] Failed to patch EnumAdapters1 vtable slot");
        }

        // Patch EnumAdapterByLuid/EnumAdapterByGpuPreference only if supported
        let factory_unknown = IUnknown::from_raw_borrowed(&factory_ptr);
        if let Some(factory_unknown) = factory_unknown {
            if factory_unknown.cast::<IDXGIFactory4>().is_ok() {
                let enum_adapter_by_luid_ok = patch_vtable_slot(
                    vtable,
                    ENUM_ADAPTER_BY_LUID_VTABLE_INDEX,
                    &ORIGINAL_ENUM_ADAPTER_BY_LUID,
                    hooked_enum_adapter_by_luid as *mut c_void,
                );
                if !enum_adapter_by_luid_ok {
                    eprintln!("[DXGI Hook] Failed to patch EnumAdapterByLuid vtable slot");
                }
            }

            if factory_unknown.cast::<IDXGIFactory6>().is_ok() {
                let enum_adapter_by_gpu_preference_ok = patch_vtable_slot(
                    vtable,
                    ENUM_ADAPTER_BY_GPU_PREFERENCE_VTABLE_INDEX,
                    &ORIGINAL_ENUM_ADAPTER_BY_GPU_PREFERENCE,
                    hooked_enum_adapter_by_gpu_preference as *mut c_void,
                );
                if !enum_adapter_by_gpu_preference_ok {
                    eprintln!("[DXGI Hook] Failed to patch EnumAdapterByGpuPreference vtable slot");
                }
            }
        }

        enum_adapters_ok || enum_adapters1_ok
    }
}

unsafe fn process_created_factory(
    factory_ptr: *mut c_void,
    factory: &IDXGIFactory1,
    target: &LUID,
) {
    if let Some(idx) = find_target_adapter_index(factory, target) {
        TARGET_ADAPTER_INDEX.store(idx, Ordering::SeqCst);

        eprintln!(
            "[DXGI Hook] Target adapter found at index {} (LUID: {}, {})",
            idx, target.HighPart, target.LowPart
        );

        if idx != 0 {
            if unsafe { patch_factory_vtable(factory_ptr) } {
                eprintln!(
                    "[DXGI Hook] Vtable patched - only adapter {} visible at index 0, others hidden",
                    idx
                );
            } else {
                eprintln!("[DXGI Hook] Warning: Failed to patch vtable");
            }
        }
    } else {
        eprintln!(
            "[DXGI Hook] Warning: No adapter found matching LUID {}, {}",
            target.HighPart, target.LowPart
        );
    }
}

fn hooked_create_dxgi_factory1(riid: *const GUID, pp_factory: *mut *mut c_void) -> HRESULT {
    let result = unsafe { CreateDXGIFactory1Hook.call(riid, pp_factory) };
    if result.is_err() {
        return result;
    }

    if let Some(target) = get_target_luid() {
        unsafe {
            if !pp_factory.is_null() && !(*pp_factory).is_null() {
                let factory_ptr = *pp_factory;
                if let Some(factory) = IDXGIFactory1::from_raw_borrowed(&factory_ptr) {
                    process_created_factory(factory_ptr, factory, target);
                }
            }
        }
    }

    result
}

fn hooked_create_dxgi_factory2(
    flags: u32,
    riid: *const GUID,
    pp_factory: *mut *mut c_void,
) -> HRESULT {
    let result = unsafe { CreateDXGIFactory2Hook.call(flags, riid, pp_factory) };
    if result.is_err() {
        return result;
    }

    if let Some(target) = get_target_luid() {
        unsafe {
            if !pp_factory.is_null() && !(*pp_factory).is_null() {
                let factory_ptr = *pp_factory;
                if let Some(factory2) = IDXGIFactory2::from_raw_borrowed(&factory_ptr)
                    && let Ok(factory1) = factory2.cast::<IDXGIFactory1>()
                {
                    process_created_factory(factory_ptr, &factory1, target);
                }
            }
        }
    }

    result
}

fn get_create_dxgi_factory1_ptr() -> Option<CreateDXGIFactory1Fn> {
    unsafe {
        let module =
            windows::Win32::System::LibraryLoader::GetModuleHandleA(windows::core::s!("dxgi.dll"))
                .ok()?;

        let proc = windows::Win32::System::LibraryLoader::GetProcAddress(
            module,
            windows::core::s!("CreateDXGIFactory1"),
        )?;

        Some(std::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            CreateDXGIFactory1Fn,
        >(proc))
    }
}

fn get_create_dxgi_factory2_ptr() -> Option<CreateDXGIFactory2Fn> {
    unsafe {
        let module =
            windows::Win32::System::LibraryLoader::GetModuleHandleA(windows::core::s!("dxgi.dll"))
                .ok()?;

        let proc = windows::Win32::System::LibraryLoader::GetProcAddress(
            module,
            windows::core::s!("CreateDXGIFactory2"),
        )?;

        Some(std::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            CreateDXGIFactory2Fn,
        >(proc))
    }
}

pub fn install_hooks(target_luid: LUID) -> bool {
    if HOOKS_INSTALLED.get().is_some() {
        return *HOOKS_INSTALLED.get().unwrap();
    }

    set_target_luid(target_luid);

    eprintln!(
        "[DXGI Hook] Installing hooks for adapter LUID: {}, {}",
        target_luid.HighPart, target_luid.LowPart
    );

    unsafe {
        let _ = windows::Win32::System::LibraryLoader::LoadLibraryA(windows::core::s!("dxgi.dll"));
    }

    let success = unsafe { install_hooks_internal() };
    HOOKS_INSTALLED.set(success).ok();

    if success {
        eprintln!("[DXGI Hook] Hooks installed successfully");
    } else {
        eprintln!("[DXGI Hook] Failed to install hooks");
    }

    success
}

unsafe fn install_hooks_internal() -> bool {
    let Some(factory1_ptr) = get_create_dxgi_factory1_ptr() else {
        eprintln!("[DXGI Hook] Failed to get CreateDXGIFactory1 address");
        return false;
    };

    let Some(factory2_ptr) = get_create_dxgi_factory2_ptr() else {
        eprintln!("[DXGI Hook] Failed to get CreateDXGIFactory2 address");
        return false;
    };

    let factory1_result = unsafe {
        CreateDXGIFactory1Hook
            .initialize(factory1_ptr, hooked_create_dxgi_factory1)
            .and_then(|_| CreateDXGIFactory1Hook.enable())
    };

    if let Err(e) = factory1_result {
        eprintln!("[DXGI Hook] Failed to hook CreateDXGIFactory1: {:?}", e);
        return false;
    }

    let factory2_result = unsafe {
        CreateDXGIFactory2Hook
            .initialize(factory2_ptr, hooked_create_dxgi_factory2)
            .and_then(|_| CreateDXGIFactory2Hook.enable())
    };

    if let Err(e) = factory2_result {
        eprintln!("[DXGI Hook] Failed to hook CreateDXGIFactory2: {:?}", e);
        unsafe {
            let _ = CreateDXGIFactory1Hook.disable();
        }
        return false;
    }

    true
}

#[allow(dead_code)]
pub fn uninstall_hooks() {
    if HOOKS_INSTALLED.get() != Some(&true) {
        return;
    }

    unsafe {
        let _ = CreateDXGIFactory1Hook.disable();
        let _ = CreateDXGIFactory2Hook.disable();
    }

    eprintln!("[DXGI Hook] Hooks uninstalled");
}
