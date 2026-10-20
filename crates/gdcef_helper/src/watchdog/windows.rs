use windows::Win32::System::Threading::{
    INFINITE, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    WaitForSingleObject,
};

pub(super) fn prepare_parent_watchdog(parent_pid: u32) -> Result<Option<usize>, ()> {
    let access = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE;
    let Ok(handle) = (unsafe { OpenProcess(access, false, parent_pid) }) else {
        eprintln!("gdcef parent process {parent_pid} exited before watchdog startup");
        return Err(());
    };
    Ok(Some(handle.0 as usize))
}

pub(super) fn wait_for_parent(_parent_pid: u32, watch: usize) {
    let handle = windows::Win32::Foundation::HANDLE(watch as *mut std::ffi::c_void);
    unsafe { WaitForSingleObject(handle, INFINITE) };
}
