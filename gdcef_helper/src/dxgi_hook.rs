//! DXGI Adapter Filtering via Detours
//!
//! This module hooks DXGI factory creation functions to filter GPU adapters,
//! ensuring CEF uses the same adapter as Godot for DX12 shared texture compatibility.
//!
//! The approach:
//! 1. Hook CreateDXGIFactory1/2 to intercept factory creation
//! 2. After the real factory is created, patch its vtable to redirect EnumAdapters1
//! 3. Our hooked EnumAdapters1 remaps adapter indices so index 0 returns our target adapter

use std::ffi::c_void;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};

use retour::static_detour;
use windows::Win32::Foundation::LUID;
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, CreateDXGIFactory2, DXGI_ADAPTER_DESC1, IDXGIAdapter1, IDXGIFactory1,
    IDXGIFactory2,
};
use windows::Win32::System::Memory::{
    PAGE_EXECUTE_READWRITE, PAGE_PROTECTION_FLAGS, VirtualProtect,
};
use windows::core::{GUID, HRESULT, Interface};

/// Target adapter LUID to filter for
static TARGET_LUID: OnceLock<LUID> = OnceLock::new();

/// Whether hooks have been successfully installed
static HOOKS_INSTALLED: OnceLock<bool> = OnceLock::new();

/// The real adapter index that matches our target LUID
static TARGET_ADAPTER_INDEX: AtomicU32 = AtomicU32::new(u32::MAX);

/// Original EnumAdapters1 function pointer (from vtable)
static ORIGINAL_ENUM_ADAPTERS1: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

// Type aliases for the hooked functions
type CreateDXGIFactory1Fn = unsafe extern "system" fn(*const GUID, *mut *mut c_void) -> HRESULT;
type CreateDXGIFactory2Fn =
    unsafe extern "system" fn(u32, *const GUID, *mut *mut c_void) -> HRESULT;

// EnumAdapters1 method signature (COM calling convention)
type EnumAdapters1Fn = unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT;

// Define static detours for factory creation
static_detour! {
    static CreateDXGIFactory1Hook: unsafe extern "system" fn(*const GUID, *mut *mut c_void) -> HRESULT;
    static CreateDXGIFactory2Hook: unsafe extern "system" fn(u32, *const GUID, *mut *mut c_void) -> HRESULT;
}

/// Sets the target adapter LUID to filter for.
pub fn set_target_luid(luid: LUID) {
    TARGET_LUID.set(luid).ok();
}

/// Gets the target adapter LUID if set.
pub fn get_target_luid() -> Option<&'static LUID> {
    TARGET_LUID.get()
}

/// Checks if an adapter matches the target LUID.
fn adapter_matches_luid(adapter: &IDXGIAdapter1, target: &LUID) -> bool {
    let mut desc = DXGI_ADAPTER_DESC1::default();
    if unsafe { adapter.GetDesc1(&mut desc) }.is_ok() {
        desc.AdapterLuid.HighPart == target.HighPart && desc.AdapterLuid.LowPart == target.LowPart
    } else {
        false
    }
}

/// Finds which real adapter index matches our target LUID.
fn find_target_adapter_index(factory: &IDXGIFactory1, target: &LUID) -> Option<u32> {
    let mut index = 0u32;
    while let Ok(adapter) = unsafe { factory.EnumAdapters1(index) } {
        if adapter_matches_luid(&adapter, target) {
            return Some(index);
        }
        index += 1;
    }
    None
}

/// Hooked EnumAdapters1 implementation.
///
/// This remaps adapter indices so that index 0 returns our target adapter,
/// and other indices return the remaining adapters in their original order.
unsafe extern "system" fn hooked_enum_adapters1(
    this: *mut c_void,
    adapter_index: u32,
    pp_adapter: *mut *mut c_void,
) -> HRESULT {
    let original: EnumAdapters1Fn =
        std::mem::transmute(ORIGINAL_ENUM_ADAPTERS1.load(Ordering::SeqCst));

    let target_index = TARGET_ADAPTER_INDEX.load(Ordering::SeqCst);

    // If we haven't found a target adapter, just pass through
    if target_index == u32::MAX {
        return original(this, adapter_index, pp_adapter);
    }

    // Remap indices:
    // - Virtual index 0 -> Real target_index (our target adapter)
    // - Virtual index 1..=target_index -> Real 0..target_index-1 (adapters before target)
    // - Virtual index > target_index -> Real index (adapters after target, unchanged)
    let real_index = if adapter_index == 0 {
        target_index
    } else if adapter_index <= target_index {
        adapter_index - 1
    } else {
        adapter_index
    };

    original(this, real_index, pp_adapter)
}

/// Gets the vtable pointer from a COM object.
unsafe fn get_vtable(obj: *mut c_void) -> *mut *mut c_void {
    *(obj as *mut *mut *mut c_void)
}

/// IDXGIFactory1 vtable layout (partial)
/// Index 7 is EnumAdapters1 in IDXGIFactory1's vtable
const ENUM_ADAPTERS1_VTABLE_INDEX: usize = 12; // IUnknown(3) + IDXGIObject(4) + IDXGIFactory(5) = 12

/// Patches the vtable of a factory to redirect EnumAdapters1.
unsafe fn patch_factory_vtable(factory_ptr: *mut c_void) -> bool {
    if factory_ptr.is_null() {
        return false;
    }

    let vtable = get_vtable(factory_ptr);
    if vtable.is_null() {
        return false;
    }

    let enum_adapters1_slot = vtable.add(ENUM_ADAPTERS1_VTABLE_INDEX);

    // Store the original function pointer (only once)
    let original = ORIGINAL_ENUM_ADAPTERS1.load(Ordering::SeqCst);
    if original.is_null() {
        ORIGINAL_ENUM_ADAPTERS1.store(*enum_adapters1_slot, Ordering::SeqCst);
    }

    // Make the vtable writable
    let mut old_protect = PAGE_PROTECTION_FLAGS(0);
    let result = VirtualProtect(
        enum_adapters1_slot as *const c_void,
        std::mem::size_of::<*mut c_void>(),
        PAGE_EXECUTE_READWRITE,
        &mut old_protect,
    );

    if result.is_err() {
        eprintln!("[DXGI Hook] Failed to change vtable protection");
        return false;
    }

    // Replace with our hook
    *enum_adapters1_slot = hooked_enum_adapters1 as *mut c_void;

    // Restore protection
    let _ = VirtualProtect(
        enum_adapters1_slot as *const c_void,
        std::mem::size_of::<*mut c_void>(),
        old_protect,
        &mut old_protect,
    );

    true
}

/// Hooked CreateDXGIFactory1 implementation.
fn hooked_create_dxgi_factory1(riid: *const GUID, pp_factory: *mut *mut c_void) -> HRESULT {
    // Call the original function
    let result = unsafe { CreateDXGIFactory1Hook.call(riid, pp_factory) };

    if result.is_err() {
        return result;
    }

    // Process the created factory
    if let Some(target) = get_target_luid() {
        unsafe {
            if !pp_factory.is_null() && !(*pp_factory).is_null() {
                let factory_ptr = *pp_factory;

                // Try to get IDXGIFactory1 interface
                if let Ok(factory) = IDXGIFactory1::from_raw_borrowed(&factory_ptr) {
                    // Find the target adapter index
                    if let Some(idx) = find_target_adapter_index(&factory, target) {
                        TARGET_ADAPTER_INDEX.store(idx, Ordering::SeqCst);

                        eprintln!(
                            "[DXGI Hook] Target adapter found at index {} (LUID: {}, {})",
                            idx, target.HighPart, target.LowPart
                        );

                        // Patch the vtable to intercept EnumAdapters1
                        if idx != 0 {
                            if patch_factory_vtable(factory_ptr) {
                                eprintln!(
                                    "[DXGI Hook] Vtable patched - adapter {} will appear at index 0",
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
            }
        }
    }

    result
}

/// Hooked CreateDXGIFactory2 implementation.
fn hooked_create_dxgi_factory2(
    flags: u32,
    riid: *const GUID,
    pp_factory: *mut *mut c_void,
) -> HRESULT {
    // Call the original function
    let result = unsafe { CreateDXGIFactory2Hook.call(flags, riid, pp_factory) };

    if result.is_err() {
        return result;
    }

    // Process the created factory (same logic as Factory1)
    if let Some(target) = get_target_luid() {
        unsafe {
            if !pp_factory.is_null() && !(*pp_factory).is_null() {
                let factory_ptr = *pp_factory;

                // IDXGIFactory2 inherits from IDXGIFactory1, so we can use the same vtable index
                if let Ok(factory2) = IDXGIFactory2::from_raw_borrowed(&factory_ptr) {
                    if let Ok(factory1) = factory2.cast::<IDXGIFactory1>() {
                        // Find the target adapter index using the original EnumAdapters1
                        // (before we patch it)
                        if let Some(idx) = find_target_adapter_index(&factory1, target) {
                            TARGET_ADAPTER_INDEX.store(idx, Ordering::SeqCst);

                            eprintln!(
                                "[DXGI Hook] Target adapter found at index {} (LUID: {}, {})",
                                idx, target.HighPart, target.LowPart
                            );

                            // Patch the vtable
                            if idx != 0 {
                                if patch_factory_vtable(factory_ptr) {
                                    eprintln!(
                                        "[DXGI Hook] Vtable patched - adapter {} will appear at index 0",
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
                }
            }
        }
    }

    result
}

/// Installs the DXGI hooks.
///
/// This should be called early in the process, before any DXGI calls are made.
/// Returns true if hooks were installed successfully.
pub fn install_hooks(target_luid: LUID) -> bool {
    // Only install once
    if HOOKS_INSTALLED.get().is_some() {
        return *HOOKS_INSTALLED.get().unwrap();
    }

    set_target_luid(target_luid);

    eprintln!(
        "[DXGI Hook] Installing hooks for adapter LUID: {}, {}",
        target_luid.HighPart, target_luid.LowPart
    );

    let success = unsafe { install_hooks_internal() };
    HOOKS_INSTALLED.set(success).ok();

    if success {
        eprintln!("[DXGI Hook] Hooks installed successfully");
    } else {
        eprintln!("[DXGI Hook] Failed to install hooks");
    }

    success
}

/// Internal hook installation (unsafe).
unsafe fn install_hooks_internal() -> bool {
    // Hook CreateDXGIFactory1
    let factory1_result = CreateDXGIFactory1Hook
        .initialize(
            CreateDXGIFactory1 as CreateDXGIFactory1Fn,
            hooked_create_dxgi_factory1,
        )
        .and_then(|_| CreateDXGIFactory1Hook.enable());

    if let Err(e) = factory1_result {
        eprintln!("[DXGI Hook] Failed to hook CreateDXGIFactory1: {:?}", e);
        return false;
    }

    // Hook CreateDXGIFactory2
    let factory2_result = CreateDXGIFactory2Hook
        .initialize(
            CreateDXGIFactory2 as CreateDXGIFactory2Fn,
            hooked_create_dxgi_factory2,
        )
        .and_then(|_| CreateDXGIFactory2Hook.enable());

    if let Err(e) = factory2_result {
        eprintln!("[DXGI Hook] Failed to hook CreateDXGIFactory2: {:?}", e);
        // Disable the first hook if the second one failed
        let _ = CreateDXGIFactory1Hook.disable();
        return false;
    }

    true
}

/// Uninstalls the DXGI hooks.
///
/// This is automatically called when the process exits, but can be called
/// manually if needed.
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
