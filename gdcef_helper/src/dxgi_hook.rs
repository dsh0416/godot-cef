//! DXGI Adapter Filtering via Detours
//!
//! This module hooks DXGI factory creation functions to filter GPU adapters,
//! ensuring CEF uses the same adapter as Godot for DX12 shared texture compatibility.
//!
//! The approach:
//! 1. Hook CreateDXGIFactory1/2 to intercept factory creation
//! 2. After the real factory is created, patch its vtable to redirect EnumAdapters and EnumAdapters1
//! 3. Our hooked functions hide all adapters except the target - only index 0 is valid

use std::ffi::c_void;
use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

use retour::static_detour;
use windows::Win32::Foundation::LUID;
use windows::Win32::Graphics::Dxgi::{IDXGIAdapter1, IDXGIFactory1, IDXGIFactory2};
use windows::Win32::System::Memory::{
    PAGE_EXECUTE_READWRITE, PAGE_PROTECTION_FLAGS, VirtualProtect,
};
use windows::core::{GUID, HRESULT, Interface};

static TARGET_LUID: OnceLock<LUID> = OnceLock::new();
static HOOKS_INSTALLED: OnceLock<bool> = OnceLock::new();
static TARGET_ADAPTER_INDEX: AtomicU32 = AtomicU32::new(u32::MAX);
static ORIGINAL_ENUM_ADAPTERS: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static ORIGINAL_ENUM_ADAPTERS1: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static VTABLE_PATCH_LOCK: Mutex<()> = Mutex::new(());

// Raw function signatures for hooking (these match the actual DLL exports)
type CreateDXGIFactory1Fn = unsafe extern "system" fn(*const GUID, *mut *mut c_void) -> HRESULT;
type CreateDXGIFactory2Fn =
    unsafe extern "system" fn(u32, *const GUID, *mut *mut c_void) -> HRESULT;

// EnumAdapters/EnumAdapters1 method signature (COM calling convention)
// Both have the same ABI signature: (this, adapter_index, pp_adapter) -> HRESULT
type EnumAdaptersFn = unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT;

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

fn adapter_matches_luid(adapter: &IDXGIAdapter1, target: &LUID) -> bool {
    match unsafe { adapter.GetDesc1() } {
        Ok(desc) => {
            desc.AdapterLuid.HighPart == target.HighPart
                && desc.AdapterLuid.LowPart == target.LowPart
        }
        Err(_) => false,
    }
}

fn find_target_adapter_index(factory: &IDXGIFactory1, target: &LUID) -> Option<u32> {
    let mut index = 0u32;
    while let Ok(adapter) = unsafe { factory.EnumAdapters1(index) } {
        if adapter_matches_luid(&adapter, target) {
            return Some(index);
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

unsafe fn get_vtable(obj: *mut c_void) -> *mut *mut c_void {
    unsafe { *(obj as *mut *mut *mut c_void) }
}

// VTable indices for EnumAdapters methods:
// IUnknown: 3 methods (QueryInterface, AddRef, Release)
// IDXGIObject: 4 methods (SetPrivateData, SetPrivateDataInterface, GetPrivateData, GetParent)
// IDXGIFactory: 4 methods (EnumAdapters, MakeWindowAssociation, GetWindowAssociation, CreateSwapChain, CreateSoftwareAdapter)
// IDXGIFactory1: 2 methods (EnumAdapters1, IsCurrent)
//
// EnumAdapters is at index 7 (3 + 4 = 7, first method of IDXGIFactory)
// EnumAdapters1 is at index 12 (3 + 4 + 5 = 12, first method of IDXGIFactory1)
const ENUM_ADAPTERS_VTABLE_INDEX: usize = 7;
const ENUM_ADAPTERS1_VTABLE_INDEX: usize = 12;

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
