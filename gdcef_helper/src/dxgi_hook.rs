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
use windows::Win32::Graphics::Dxgi::{IDXGIAdapter1, IDXGIFactory1, IDXGIFactory2};
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

// Raw function signatures for hooking (these match the actual DLL exports)
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
    match unsafe { adapter.GetDesc1() } {
        Ok(desc) => {
            desc.AdapterLuid.HighPart == target.HighPart
                && desc.AdapterLuid.LowPart == target.LowPart
        }
        Err(_) => false,
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
    unsafe {
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
}

/// Gets the vtable pointer from a COM object.
unsafe fn get_vtable(obj: *mut c_void) -> *mut *mut c_void {
    unsafe { *(obj as *mut *mut *mut c_void) }
}

/// IDXGIFactory1 vtable layout (partial)
/// Index 12 is EnumAdapters1 in IDXGIFactory1's vtable
/// IUnknown(3) + IDXGIObject(4) + IDXGIFactory(5) = 12
const ENUM_ADAPTERS1_VTABLE_INDEX: usize = 12;

/// RAII guard for memory protection changes.
/// Ensures protection is restored when dropped, even on panic.
struct MemoryProtectionGuard {
    address: *const c_void,
    size: usize,
    old_protect: PAGE_PROTECTION_FLAGS,
    active: bool,
}

impl MemoryProtectionGuard {
    /// Creates a new guard that will restore protection on drop.
    /// Returns None if changing protection failed.
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

    /// Restores the original protection and marks the guard as inactive.
    /// Returns true if restoration succeeded.
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
            // Safety: We're in drop, so we must restore protection.
            // If this fails, we log a warning but can't do much else.
            let mut dummy = PAGE_PROTECTION_FLAGS(0);
            let result =
                unsafe { VirtualProtect(self.address, self.size, self.old_protect, &mut dummy) };
            if result.is_err() {
                eprintln!(
                    "[DXGI Hook] Warning: Failed to restore memory protection in drop. \
                     Memory at {:p} may remain writable+executable.",
                    self.address
                );
            }
        }
    }
}

/// Patches the vtable of a factory to redirect EnumAdapters1.
unsafe fn patch_factory_vtable(factory_ptr: *mut c_void) -> bool {
    unsafe {
        if factory_ptr.is_null() {
            return false;
        }

        let vtable = get_vtable(factory_ptr);
        if vtable.is_null() {
            return false;
        }

        let enum_adapters1_slot = vtable.add(ENUM_ADAPTERS1_VTABLE_INDEX);

        // Store the original function pointer (only once)
        let _ = ORIGINAL_ENUM_ADAPTERS1.compare_exchange(
            std::ptr::null_mut(),
            *enum_adapters1_slot,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );

        // Make the vtable writable using RAII guard for automatic cleanup
        let slot_ptr = enum_adapters1_slot as *const c_void;
        let slot_size = std::mem::size_of::<*mut c_void>();

        let Some(mut guard) = MemoryProtectionGuard::new(slot_ptr, slot_size) else {
            eprintln!("[DXGI Hook] Failed to change vtable protection");
            return false;
        };

        // Replace with our hook - this is the only operation in the vulnerable window
        *enum_adapters1_slot = hooked_enum_adapters1 as *mut c_void;

        // Explicitly restore protection and check result
        if !guard.restore() {
            eprintln!(
                "[DXGI Hook] Warning: Failed to restore vtable protection. \
                 Memory may remain writable+executable."
            );
            // Still return true since the hook was installed successfully
        }

        true
    }
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
                if let Some(factory) = IDXGIFactory1::from_raw_borrowed(&factory_ptr) {
                    // Find the target adapter index
                    if let Some(idx) = find_target_adapter_index(factory, target) {
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
                if let Some(factory2) = IDXGIFactory2::from_raw_borrowed(&factory_ptr) {
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

/// Gets the raw function pointer for CreateDXGIFactory1 from dxgi.dll
fn get_create_dxgi_factory1_ptr() -> Option<CreateDXGIFactory1Fn> {
    unsafe {
        let module =
            windows::Win32::System::LibraryLoader::GetModuleHandleA(windows::core::s!("dxgi.dll"))
                .ok()?;

        let proc = windows::Win32::System::LibraryLoader::GetProcAddress(
            module,
            windows::core::s!("CreateDXGIFactory1"),
        )?;

        Some(std::mem::transmute(proc))
    }
}

/// Gets the raw function pointer for CreateDXGIFactory2 from dxgi.dll
fn get_create_dxgi_factory2_ptr() -> Option<CreateDXGIFactory2Fn> {
    unsafe {
        let module =
            windows::Win32::System::LibraryLoader::GetModuleHandleA(windows::core::s!("dxgi.dll"))
                .ok()?;

        let proc = windows::Win32::System::LibraryLoader::GetProcAddress(
            module,
            windows::core::s!("CreateDXGIFactory2"),
        )?;

        Some(std::mem::transmute(proc))
    }
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

    // Load dxgi.dll first to ensure it's available
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

/// Internal hook installation (unsafe).
unsafe fn install_hooks_internal() -> bool {
    // Get the raw function pointers
    let Some(factory1_ptr) = get_create_dxgi_factory1_ptr() else {
        eprintln!("[DXGI Hook] Failed to get CreateDXGIFactory1 address");
        return false;
    };

    let Some(factory2_ptr) = get_create_dxgi_factory2_ptr() else {
        eprintln!("[DXGI Hook] Failed to get CreateDXGIFactory2 address");
        return false;
    };

    // Hook CreateDXGIFactory1
    let factory1_result = CreateDXGIFactory1Hook
        .initialize(factory1_ptr, hooked_create_dxgi_factory1)
        .and_then(|_| CreateDXGIFactory1Hook.enable());

    if let Err(e) = factory1_result {
        eprintln!("[DXGI Hook] Failed to hook CreateDXGIFactory1: {:?}", e);
        return false;
    }

    // Hook CreateDXGIFactory2
    let factory2_result = CreateDXGIFactory2Hook
        .initialize(factory2_ptr, hooked_create_dxgi_factory2)
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
