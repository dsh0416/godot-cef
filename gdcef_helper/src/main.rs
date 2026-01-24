#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

use cef::{CefString, ImplCommandLine, api_hash, args::Args, execute_process};

// In Godot's codebase, Godot sets NvOptimusEnablement and AmdPowerXpressRequestHighPerformance
// to request discrete GPU on Windows laptops with hybrid graphics.
// This might cause the gdcef_helper uses a different GPU than Godot.
// See https://github.com/godotengine/godot/blob/741fb8a30687d0662ab6b5c04a2a531440dd29d9/platform/windows/os_windows.cpp#L101
#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
#[used]
pub static NvOptimusEnablement: u32 = 0x00000001;

#[cfg(target_os = "windows")]
#[unsafe(no_mangle)]
#[used]
pub static AmdPowerXpressRequestHighPerformance: u32 = 0x00000001;

mod utils;

fn main() -> std::process::ExitCode {
    #[cfg(target_os = "macos")]
    {
        let framework_path = utils::get_framework_path().expect("Failed to get CEF framework path");
        cef_app::load_cef_framework_from_path(&framework_path);
    }

    api_hash(cef::sys::CEF_API_VERSION_LAST, 0);

    let args = Args::new();
    let cmd = args.as_cmd_line().unwrap();

    #[cfg(target_os = "macos")]
    {
        let framework_path = utils::get_framework_path().expect("Failed to get CEF framework path");
        cef_app::load_sandbox_from_path(&framework_path, args.as_main_args());
    }

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
