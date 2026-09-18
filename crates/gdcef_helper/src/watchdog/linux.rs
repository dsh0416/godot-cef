pub(super) fn prepare_parent_watchdog(parent_pid: u32) -> Result<Option<usize>, ()> {
    let result = unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) };
    if result != 0 {
        eprintln!("Failed to arm gdcef Linux parent-death signal");
        return Err(());
    }

    if !is_parent_alive(parent_pid, 0) {
        eprintln!("gdcef parent process {parent_pid} exited before watchdog startup");
        return Err(());
    }
    Ok(Some(0))
}

pub(super) fn is_parent_alive(parent_pid: u32, _watch: usize) -> bool {
    unsafe { libc::getppid() as u32 == parent_pid }
}
