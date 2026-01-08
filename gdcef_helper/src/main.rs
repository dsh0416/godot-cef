#[cfg(target_os = "macos")]
use cef::MainArgs;
use cef::{CefString, ImplCommandLine, api_hash, args::Args, execute_process};

mod utils;

fn load_cef_framework() {
    #[cfg(target_os = "macos")]
    {
        use cef::sys::cef_load_library;

        let framework_path = utils::get_framework_path();
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
    };
}

#[cfg(all(target_os = "macos"))]
fn load_sandbox(args: &MainArgs) {
    use libloading::Library;

    let framework_path = utils::get_framework_path();
    let path = framework_path
        .unwrap()
        .join("Libraries/libcef_sandbox.dylib")
        .canonicalize()
        .unwrap();

    unsafe {
        let lib = Library::new(path).unwrap();
        let func =
            lib.get::<unsafe extern "C" fn(
                argc: std::os::raw::c_int,
                argv: *mut *mut ::std::os::raw::c_char,
            )>(b"cef_sandbox_initialize\0")
                .unwrap();
        func(args.argc, args.argv);
    }
}

fn main() -> std::process::ExitCode {
    load_cef_framework();
    api_hash(cef::sys::CEF_API_VERSION_LAST, 0);

    let args = Args::new();
    let cmd = args.as_cmd_line().unwrap();

    // This is required only for sandbox mode on macOS
    #[cfg(all(target_os = "macos"))]
    load_sandbox(args.as_main_args());

    let switch = CefString::from("type");
    let is_browser_process = cmd.has_switch(Some(&switch)) != 1;
    let mut app = cef_app::AppBuilder::build(cef_app::OsrApp::new());
    let ret = execute_process(
        Some(args.as_main_args()),
        Some(&mut app),
        std::ptr::null_mut(),
    );

    if is_browser_process {
        assert!(ret == -1, "cannot execute browser process");
    } else {
        let process_type = CefString::from(&cmd.switch_value(Some(&switch)));
        println!("launch process {process_type}");
        assert!(ret >= 0, "cannot execute non-browser process");
        // non-browser process does not initialize cef
        return 0.into();
    }

    std::process::ExitCode::SUCCESS
}
