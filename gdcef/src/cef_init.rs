use cef::Settings;
use godot::classes::Os;
use godot::prelude::*;
use std::path::PathBuf;
use std::sync::Once;

#[cfg(target_os = "macos")]
use cef::api_hash;
#[cfg(target_os = "macos")]
use crate::utils::get_framework_path;
use crate::utils::get_subprocess_path;

use crate::accelerated_osr::RenderBackend;

/// Global initialization guard - CEF can only be initialized once
pub static CEF_INITIALIZED: Once = Once::new();

/// Loads the CEF framework library (macOS-specific)
#[cfg(target_os = "macos")]
pub fn load_cef_framework() {
    use cef::sys::cef_load_library;

    let framework_path = get_framework_path();
    let path = framework_path
        .unwrap()
        .join("Chromium Embedded Framework")
        .canonicalize()
        .unwrap();

    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        panic!("Failed to convert library path to CString");
    };
    let result = unsafe {
        let arg_path = Some(&*path.as_ptr().cast());
        let arg_path = arg_path.map(std::ptr::from_ref).unwrap_or(std::ptr::null());
        cef_load_library(arg_path) == 1
    };

    assert!(result, "Failed to load macOS CEF framework");

    // set the API hash
    let _ = api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
}

#[cfg(not(target_os = "macos"))]
pub fn load_cef_framework() {
    // No-op on other platforms
}

/// Loads the CEF sandbox (macOS-specific)
#[cfg(target_os = "macos")]
pub fn load_sandbox(args: &cef::MainArgs) {
    use libloading::Library;

    let framework_path = get_framework_path();
    let path = framework_path
        .unwrap()
        .join("Libraries/libcef_sandbox.dylib")
        .canonicalize()
        .unwrap();

    unsafe {
        let lib = Library::new(path).unwrap();
        let func = lib
            .get::<unsafe extern "C" fn(
                argc: std::os::raw::c_int,
                argv: *mut *mut ::std::os::raw::c_char,
            )>(b"cef_sandbox_initialize\0")
            .unwrap();
        func(args.argc, args.argv);
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

    let subprocess_path = get_subprocess_path().unwrap();

    godot_print!("subprocess_path: {}", subprocess_path.to_str().unwrap());

    let user_data_dir = PathBuf::from(Os::singleton().get_user_data_dir().to_string());
    let root_cache_path = user_data_dir.join("Godot CEF/Cache");

    let settings = Settings {
        browser_subprocess_path: subprocess_path.to_str().unwrap().into(),
        windowless_rendering_enabled: true as _,
        external_message_pump: true as _,
        log_severity: cef::LogSeverity::VERBOSE as _,
        root_cache_path: root_cache_path.to_str().unwrap().into(),
        ..Default::default()
    };

    #[cfg(target_os = "macos")]
    let settings = Settings {
        framework_dir_path: get_framework_path().unwrap().to_str().unwrap().into(),
        main_bundle_path: get_subprocess_path()
            .unwrap()
            .join("../../..")
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
            .into(),
        ..settings
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

