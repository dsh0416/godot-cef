use std::time::Duration;

use tokio::runtime::Builder;
use tokio::time::{MissedTickBehavior, interval};

const WATCHDOG_INTERVAL: Duration = Duration::from_secs(1);

pub(super) fn spawn_parent_watchdog(parent_pid: u32) {
    let thread = std::thread::Builder::new().name("gdcef-parent-watchdog".to_string());
    if let Err(err) = thread.spawn(move || {
        let runtime = match Builder::new_current_thread().enable_time().build() {
            Ok(runtime) => runtime,
            Err(err) => {
                eprintln!("Failed to start gdcef parent watchdog runtime: {err}");
                return;
            }
        };

        runtime.block_on(async move {
            let mut ticker = interval(WATCHDOG_INTERVAL);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

            loop {
                ticker.tick().await;
                if !super::platform::is_parent_alive(parent_pid) {
                    eprintln!("gdcef parent process {parent_pid} exited; helper is exiting");
                    std::process::exit(0);
                }
            }
        });
    }) {
        eprintln!("Failed to spawn gdcef parent watchdog thread: {err}");
    }
}
