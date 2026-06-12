use std::path::Path;

pub(super) fn prepare_parent_watchdog(parent_pid: u32) {
    let result = unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) };
    if result != 0 {
        eprintln!("Failed to arm gdcef Linux parent-death signal");
    }

    if !is_parent_alive(parent_pid) {
        eprintln!("gdcef parent process {parent_pid} exited before watchdog startup");
        std::process::exit(0);
    }
}

pub(super) fn is_parent_alive(parent_pid: u32) -> bool {
    Path::new("/proc").join(parent_pid.to_string()).exists()
}
