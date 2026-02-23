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
        let framework_path = match utils::get_framework_path() {
            Ok(path) => path,
            Err(err) => {
                eprintln!("Failed to get CEF framework path: {err}");
                return std::process::ExitCode::FAILURE;
            }
        };
        if let Err(err) = cef_app::load_cef_framework_from_path(&framework_path) {
            eprintln!("Failed to load CEF framework: {err}");
            return std::process::ExitCode::FAILURE;
        }
    }

    api_hash(cef::sys::CEF_API_VERSION_LAST, 0);

    let args = Args::new();
    let Some(cmd) = args.as_cmd_line() else {
        eprintln!("Failed to parse CEF command line args");
        return std::process::ExitCode::FAILURE;
    };

    #[cfg(target_os = "macos")]
    {
        let framework_path = match utils::get_framework_path() {
            Ok(path) => path,
            Err(err) => {
                eprintln!("Failed to get CEF framework path: {err}");
                return std::process::ExitCode::FAILURE;
            }
        };
        if let Err(err) = cef_app::load_sandbox_from_path(&framework_path, args.as_main_args()) {
            eprintln!("Failed to load CEF sandbox: {err}");
            return std::process::ExitCode::FAILURE;
        }
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
        if ret != -1 {
            eprintln!("cannot execute browser process");
            return std::process::ExitCode::FAILURE;
        }
    } else {
        let process_type = CefString::from(&cmd.switch_value(Some(&switch)));
        println!("launch process {process_type}");
        if ret < 0 {
            eprintln!("cannot execute non-browser process");
            return std::process::ExitCode::FAILURE;
        }
        // non-browser process does not initialize cef
        return std::process::ExitCode::SUCCESS;
    }

    std::process::ExitCode::SUCCESS
}
