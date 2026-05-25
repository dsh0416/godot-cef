use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

const STILL_ACTIVE_EXIT_CODE: u32 = 259;

pub(super) fn prepare_parent_watchdog(parent_pid: u32) -> Result<Option<usize>, ()> {
    let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, parent_pid) })
    else {
        eprintln!("gdcef parent process {parent_pid} exited before watchdog startup");
        return Err(());
    };
    if !is_parent_alive(parent_pid, handle.0 as usize) {
        unsafe { CloseHandle(handle) };
        eprintln!("gdcef parent process {parent_pid} exited before watchdog startup");
        return Err(());
    }
    Ok(Some(handle.0 as usize))
}

pub(super) fn is_parent_alive(_parent_pid: u32, watch: usize) -> bool {
    let handle = windows::Win32::Foundation::HANDLE(watch as isize);
    let mut exit_code = 0;
    let is_alive = unsafe { GetExitCodeProcess(handle, &mut exit_code) }.is_ok()
        && exit_code == STILL_ACTIVE_EXIT_CODE;
    is_alive
}
