pub(super) fn prepare_parent_watchdog(_parent_pid: u32) {
    eprintln!("gdcef parent watchdog is not supported on this platform");
}

pub(super) fn is_parent_alive(_parent_pid: u32) -> bool {
    true
}
