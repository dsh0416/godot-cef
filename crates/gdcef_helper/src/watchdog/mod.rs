mod runtime;

#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod platform;

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod platform;

#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod platform;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
#[path = "unsupported.rs"]
mod platform;

use cef::{CefString, CommandLine, ImplCommandLine};

pub(crate) fn start_parent_watchdog(command_line: &CommandLine) {
    let switch = CefString::from(cef_app::GDCEF_PARENT_PID_SWITCH);
    if command_line.has_switch(Some(&switch)) != 1 {
        return;
    }

    let parent_pid = CefString::from(&command_line.switch_value(Some(&switch))).to_string();
    let Ok(parent_pid) = parent_pid.parse::<u32>() else {
        eprintln!(
            "Ignoring invalid {} switch value",
            cef_app::GDCEF_PARENT_PID_SWITCH
        );
        return;
    };

    if parent_pid == 0 || parent_pid == std::process::id() {
        eprintln!(
            "Ignoring unusable {} switch value: {}",
            cef_app::GDCEF_PARENT_PID_SWITCH,
            parent_pid
        );
        return;
    }

    platform::prepare_parent_watchdog(parent_pid);
    runtime::spawn_parent_watchdog(parent_pid);
}
