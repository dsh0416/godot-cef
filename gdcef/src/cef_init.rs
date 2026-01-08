use cef::Settings;
use godot::classes::Os;
use godot::prelude::*;
use std::path::PathBuf;
use std::sync::Once;

#[cfg(target_os = "macos")]
use crate::utils::get_framework_path;
use crate::utils::get_subprocess_path;

use crate::accelerated_osr::RenderBackend;

/// Global initialization guard - CEF can only be initialized once
pub static CEF_INITIALIZED: Once = Once::new();

/// Loads the CEF framework library (macOS-specific)
#[cfg(target_os = "macos")]
pub fn load_cef_framework() {
    match get_framework_path() {
        Ok(framework_path) => cef_app::load_cef_framework_from_path(&framework_path),
        Err(e) => panic!("Failed to get CEF framework path: {}", e),
    }
}

#[cfg(not(target_os = "macos"))]
pub fn load_cef_framework() {
    // No-op on other platforms
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

/// Initializes CEF with the given settings
pub fn initialize_cef() {
    let args = cef::args::Args::new();
    let godot_backend = detect_godot_render_backend();
    let mut app = cef_app::AppBuilder::build(cef_app::OsrApp::with_godot_backend(godot_backend));

    #[cfg(target_os = "macos")]
    load_sandbox(args.as_main_args());

    let subprocess_path = match get_subprocess_path() {
        Ok(path) => path,
        Err(e) => panic!("Failed to get subprocess path: {}", e),
    };

    let user_data_dir = PathBuf::from(Os::singleton().get_user_data_dir().to_string());
    let root_cache_path = user_data_dir.join("Godot CEF/Cache");

    let settings = Settings {
        browser_subprocess_path: subprocess_path
            .to_str()
            .expect("subprocess path is not valid UTF-8")
            .into(),
        windowless_rendering_enabled: true as _,
        external_message_pump: true as _,
        log_severity: cef::LogSeverity::DEFAULT as _,
        root_cache_path: root_cache_path
            .to_str()
            .expect("cache path is not valid UTF-8")
            .into(),
        ..Default::default()
    };

    #[cfg(target_os = "macos")]
    let settings = {
        let framework_path = get_framework_path().expect("Failed to get framework path");
        let main_bundle_path = get_subprocess_path()
            .expect("Failed to get subprocess path")
            .join("../../..")
            .canonicalize()
            .expect("Failed to canonicalize main bundle path");

        Settings {
            framework_dir_path: framework_path
                .to_str()
                .expect("framework path is not valid UTF-8")
                .into(),
            main_bundle_path: main_bundle_path
                .to_str()
                .expect("main bundle path is not valid UTF-8")
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

    assert_eq!(ret, 1, "failed to initialize CEF");
}

/// Shuts down CEF if it was initialized
pub fn shutdown_cef() {
    if CEF_INITIALIZED.is_completed() {
        cef::shutdown();
    }
}
