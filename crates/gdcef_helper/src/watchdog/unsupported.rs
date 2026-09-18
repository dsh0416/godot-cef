pub(super) fn prepare_parent_watchdog(_parent_pid: u32) -> Result<Option<usize>, ()> {
    eprintln!("gdcef parent watchdog is not supported on this platform");
    Ok(None)
}

pub(super) fn wait_for_parent(_parent_pid: u32, _watch: usize) {}
