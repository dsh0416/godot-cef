use cef::Settings;
use godot::classes::{Engine, Os};
use godot::prelude::*;
use std::sync::{Mutex, MutexGuard};

#[cfg(target_os = "macos")]
use crate::utils::get_framework_path;
use crate::utils::get_subprocess_path;

use crate::accelerated_osr::RenderBackend;
use crate::error::{CefError, CefResult};
use crate::settings;

struct CefState {
    ref_count: usize,
    initialized: bool,
}

static CEF_STATE: Mutex<CefState> = Mutex::new(CefState {
    ref_count: 0,
    initialized: false,
});

fn lock_cef_state() -> MutexGuard<'static, CefState> {
    match CEF_STATE.lock() {
        Ok(state) => state,
        Err(poisoned) => {
            godot::global::godot_warn!(
                "[CefInit] CEF state mutex was poisoned; continuing with recovered state"
            );
            poisoned.into_inner()
        }
    }
}

pub fn cef_retain() -> CefResult<()> {
    let mut state = lock_cef_state();

    if state.ref_count == 0 {
        load_cef_framework()?;
        cef::api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
        initialize_cef()?;
        state.initialized = true;

        settings::warn_if_insecure_settings();
        settings::log_production_security_baseline();
    }

    state.ref_count += 1;
    Ok(())
}

pub fn cef_release() {
    let mut state = lock_cef_state();

    if state.ref_count == 0 {
        return;
    }

    state.ref_count -= 1;

    if state.ref_count == 0 && state.initialized {
        cef::shutdown();
        state.initialized = false;
    }
}

/// Loads the CEF framework library (macOS-specific)
#[cfg(target_os = "macos")]
fn load_cef_framework() -> CefResult<()> {
    let framework_path = get_framework_path().map_err(|e| {
        CefError::FrameworkLoadFailed(format!("Failed to get CEF framework path: {}", e))
    })?;
    cef_app::load_cef_framework_from_path(&framework_path);
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn load_cef_framework() -> CefResult<()> {
    // No-op on other platforms
    Ok(())
}

/// Loads the CEF sandbox (macOS-specific)
#[cfg(target_os = "macos")]
fn load_sandbox(args: &cef::MainArgs) {
    match get_framework_path() {
        Ok(framework_path) => cef_app::load_sandbox_from_path(&framework_path, args),
        Err(e) => godot::global::godot_warn!("Failed to load CEF sandbox: {}", e),
    }
}

fn detect_godot_render_backend() -> cef_app::GodotRenderBackend {
    let godot_backend = RenderBackend::detect();

    match godot_backend {
        RenderBackend::Metal => cef_app::GodotRenderBackend::Metal,
        RenderBackend::Vulkan => cef_app::GodotRenderBackend::Vulkan,
        RenderBackend::D3D12 => cef_app::GodotRenderBackend::Direct3D12,
        _ => cef_app::GodotRenderBackend::Unknown,
    }
}

/// Determines if remote debugging should be enabled.
///
/// Remote debugging is only enabled when:
/// - Godot is compiled in debug mode (OS.is_debug_build() returns true), OR
/// - The game is running from the Godot editor (Engine.is_editor_hint() returns true)
///
/// This is a security measure to prevent remote debugging in production builds.
fn should_enable_remote_debugging() -> bool {
    let os = Os::singleton();
    let engine = Engine::singleton();

    let is_debug_build = os.is_debug_build();
    let is_editor_hint = engine.is_editor_hint();

    is_debug_build || is_editor_hint
}

/// Initializes CEF with the given settings
fn initialize_cef() -> CefResult<()> {
    let args = cef::args::Args::new();
    let godot_backend = detect_godot_render_backend();
    let (accel_supported, accel_reason) =
        crate::accelerated_osr::accelerated_osr_support_diagnostic();
    let enable_remote_debugging = should_enable_remote_debugging();
    let remote_debugging_port = settings::get_remote_devtools_port();

    let security_config = settings::get_security_config();
    let user_agent = settings::get_user_agent();
    let proxy_server = settings::get_proxy_server();
    let proxy_bypass_list = settings::get_proxy_bypass_list();
    let cache_size_mb = settings::get_cache_size_mb();
    let custom_switches = settings::get_custom_switches();

    godot::global::godot_print!(
        "[CefInit] Startup summary: backend={:?}, accelerated_osr_supported={}, reason={}, remote_debugging={}, remote_port={}, cache_size_mb={}",
        godot_backend,
        accel_supported,
        accel_reason,
        enable_remote_debugging,
        remote_debugging_port,
        cache_size_mb
    );

    #[allow(unused_mut)]
    let mut app_builder = cef_app::OsrApp::builder()
        .godot_backend(godot_backend)
        .remote_debugging(enable_remote_debugging)
        .remote_debugging_port(remote_debugging_port)
        .security_config(security_config)
        .user_agent(user_agent)
        .proxy_server(proxy_server)
        .proxy_bypass_list(proxy_bypass_list)
        .cache_size_mb(cache_size_mb)
        .custom_switches(custom_switches);

    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    {
        use crate::accelerated_osr::get_godot_gpu_device_ids;
        if let Some((vendor_id, device_id)) = get_godot_gpu_device_ids() {
            godot::global::godot_print!(
                "[CefInit] Godot GPU: vendor=0x{:04x}, device=0x{:04x} - will pass to CEF subprocesses",
                vendor_id,
                device_id
            );
            app_builder = app_builder.gpu_device_ids(vendor_id, device_id);
        }
    }

    let mut app = cef_app::AppBuilder::build(app_builder.build());

    #[cfg(target_os = "macos")]
    load_sandbox(args.as_main_args());

    let subprocess_path = get_subprocess_path().map_err(|e| {
        CefError::InitializationFailed(format!("Failed to get subprocess path: {}", e))
    })?;

    let root_cache_path = settings::get_data_path();

    let settings = Settings {
        browser_subprocess_path: subprocess_path
            .to_str()
            .ok_or_else(|| {
                CefError::InitializationFailed("subprocess path is not valid UTF-8".to_string())
            })?
            .into(),
        windowless_rendering_enabled: true as _,
        external_message_pump: true as _,
        log_severity: cef::LogSeverity::DEFAULT as _,
        root_cache_path: root_cache_path
            .to_str()
            .ok_or_else(|| {
                CefError::InitializationFailed("cache path is not valid UTF-8".to_string())
            })?
            .into(),
        ..Default::default()
    };

    #[cfg(target_os = "macos")]
    let settings = {
        let framework_path = get_framework_path().map_err(|e| {
            CefError::InitializationFailed(format!("Failed to get framework path: {}", e))
        })?;
        let main_bundle_path = get_subprocess_path()
            .map_err(|e| {
                CefError::InitializationFailed(format!("Failed to get subprocess path: {}", e))
            })?
            .join("../../..")
            .canonicalize()
            .map_err(|e| {
                CefError::InitializationFailed(format!(
                    "Failed to canonicalize main bundle path: {}",
                    e
                ))
            })?;

        Settings {
            framework_dir_path: framework_path
                .to_str()
                .ok_or_else(|| {
                    CefError::InitializationFailed("framework path is not valid UTF-8".to_string())
                })?
                .into(),
            main_bundle_path: main_bundle_path
                .to_str()
                .ok_or_else(|| {
                    CefError::InitializationFailed(
                        "main bundle path is not valid UTF-8".to_string(),
                    )
                })?
                .into(),
            ..settings
        }
    };

    let ret = cef::initialize(
        Some(args.as_main_args()),
        Some(&settings),
        Some(&mut app),
        std::ptr::null_mut(),
    );

    if ret != 1 {
        return Err(CefError::InitializationFailed(
            "CEF initialization returned error code".to_string(),
        ));
    }

    Ok(())
}
