use cef::Settings;
use godot::classes::{Engine, Os};
use godot::prelude::*;
use std::path::PathBuf;
use std::sync::Mutex;

#[cfg(target_os = "macos")]
use crate::utils::get_framework_path;
use crate::utils::get_subprocess_path;

use crate::accelerated_osr::RenderBackend;
use crate::error::{CefError, CefResult};
use cef_app::SecurityConfig;

struct CefState {
    ref_count: usize,
    initialized: bool,
}

static CEF_STATE: Mutex<CefState> = Mutex::new(CefState {
    ref_count: 0,
    initialized: false,
});

pub fn cef_retain_with_security(security_config: SecurityConfig) -> CefResult<()> {
    let mut state = CEF_STATE.lock().unwrap();

    if state.ref_count == 0 {
        load_cef_framework()?;
        cef::api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
        initialize_cef(security_config)?;
        state.initialized = true;
    }

    state.ref_count += 1;
    Ok(())
}

pub fn cef_release() {
    let mut state = CEF_STATE.lock().unwrap();

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
fn initialize_cef(security_config: SecurityConfig) -> CefResult<()> {
    let args = cef::args::Args::new();
    let godot_backend = detect_godot_render_backend();
    let enable_remote_debugging = should_enable_remote_debugging();

    #[allow(unused_mut)]
    let mut osr_app = cef_app::OsrApp::with_security_options(
        godot_backend,
        enable_remote_debugging,
        security_config,
    );

    // To make sure CEF uses the correct GPU adapter,
    // we need to pass the adapter LUID to the subprocesses.
    #[cfg(target_os = "windows")]
    {
        use crate::accelerated_osr::get_godot_adapter_luid;
        if let Some((high, low)) = get_godot_adapter_luid() {
            godot::global::godot_print!(
                "[CefInit] Godot adapter LUID: {},{} - will pass to CEF subprocesses",
                high,
                low
            );
            osr_app = osr_app.with_adapter_luid(high, low);
        }
    }

    // On Linux, pass the device UUID to ensure CEF uses the same GPU as Godot.
    #[cfg(target_os = "linux")]
    {
        use crate::accelerated_osr::get_godot_device_uuid;
        if let Some(uuid) = get_godot_device_uuid() {
            godot::global::godot_print!(
                "[CefInit] Godot device UUID retrieved - will pass to CEF subprocesses"
            );
            osr_app = osr_app.with_device_uuid(uuid);
        }
    }

    let mut app = cef_app::AppBuilder::build(osr_app);

    #[cfg(target_os = "macos")]
    load_sandbox(args.as_main_args());

    let subprocess_path = get_subprocess_path().map_err(|e| {
        CefError::InitializationFailed(format!("Failed to get subprocess path: {}", e))
    })?;

    let user_data_dir = PathBuf::from(Os::singleton().get_user_data_dir().to_string());
    let root_cache_path = user_data_dir.join("Godot CEF/Cache");

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
