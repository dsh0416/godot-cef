pub(super) fn prepare_parent_watchdog(parent_pid: u32) {
    if !is_parent_alive(parent_pid) {
        eprintln!("gdcef parent process {parent_pid} exited before watchdog startup");
        std::process::exit(0);
    }
}

pub(super) fn is_parent_alive(parent_pid: u32) -> bool {
    unsafe { libc::kill(parent_pid as libc::pid_t, 0) == 0 }
}
