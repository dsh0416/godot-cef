pub(super) fn prepare_parent_watchdog(parent_pid: u32) -> Result<Option<usize>, ()> {
    if !is_parent_alive(parent_pid, 0) {
        eprintln!("gdcef parent process {parent_pid} exited before watchdog startup");
        return Err(());
    }
    Ok(Some(0))
}

pub(super) fn is_parent_alive(parent_pid: u32, _watch: usize) -> bool {
    unsafe { libc::getppid() as u32 == parent_pid }
}

pub(super) fn wait_for_parent(parent_pid: u32, _watch: usize) {
    while is_parent_alive(parent_pid, 0) {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
