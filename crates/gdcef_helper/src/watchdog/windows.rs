use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

const STILL_ACTIVE_EXIT_CODE: u32 = 259;

pub(super) fn prepare_parent_watchdog(parent_pid: u32) {
    if !is_parent_alive(parent_pid) {
        eprintln!("gdcef parent process {parent_pid} exited before watchdog startup");
        std::process::exit(0);
    }
}

pub(super) fn is_parent_alive(parent_pid: u32) -> bool {
    let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, parent_pid) })
    else {
        return false;
    };

    let mut exit_code = 0;
    let is_alive = unsafe { GetExitCodeProcess(handle, &mut exit_code) }.is_ok()
        && exit_code == STILL_ACTIVE_EXIT_CODE;
    let _ = unsafe { CloseHandle(handle) };
    is_alive
}
