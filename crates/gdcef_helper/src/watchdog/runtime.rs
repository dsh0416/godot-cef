use std::time::Duration;

const WATCHDOG_INTERVAL: Duration = Duration::from_secs(1);

pub(super) fn spawn_parent_watchdog(parent_pid: u32, watch: usize) {
    let thread = std::thread::Builder::new().name("gdcef-parent-watchdog".to_string());
    if let Err(err) = thread.spawn(move || {
        loop {
            std::thread::sleep(WATCHDOG_INTERVAL);
            if !super::platform::is_parent_alive(parent_pid, watch) {
                eprintln!("gdcef parent process {parent_pid} exited; helper is exiting");
                std::process::exit(0);
            }
        }
    }) {
        eprintln!("Failed to spawn gdcef parent watchdog thread: {err}");
    }
}
