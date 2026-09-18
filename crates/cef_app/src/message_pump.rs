//! Thread-safe scheduling for CEF's external UI message pump.
//!
//! CEF may schedule work from any thread. Only the thread that activates the
//! scheduler may execute it, and the scheduling lock is never held across CEF.

use std::sync::{Mutex, MutexGuard};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

// CEF's reference external pump also limits idle polling to 30 Hz to keep native
// work moving when no Chromium task schedules a wakeup. The host event loop may
// service this later; this is not a separate timer thread.
const MAX_IDLE_DELAY: Duration = Duration::from_millis(1000 / 30);

pub static MESSAGE_PUMP: MessagePumpScheduler = MessagePumpScheduler::new();

pub struct MessagePumpScheduler {
    state: Mutex<PumpState>,
}

struct PumpState {
    owner: Option<ThreadId>,
    deadline: Option<Instant>,
    running: bool,
}

impl MessagePumpScheduler {
    pub const fn new() -> Self {
        Self {
            state: Mutex::new(PumpState {
                owner: None,
                deadline: None,
                running: false,
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, PumpState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Activate after successful CEF initialization on the application's UI
    /// thread. Repeated activation does not change the owner or schedule.
    pub fn activate(&self) {
        let mut state = self.lock();
        if state.owner.is_none() {
            state.owner = Some(thread::current().id());
            state.deadline = Some(Instant::now());
        }
    }

    /// Disable host callbacks before tearing down the host event loop.
    pub fn deactivate(&self) {
        let mut state = self.lock();
        state.owner = None;
        state.deadline = None;
    }

    /// Called by OnScheduleMessagePumpWork, including from CEF worker threads.
    /// A new request replaces the previous deadline, per CEF's contract.
    pub fn schedule(&self, delay_ms: i64) {
        self.schedule_at(Instant::now(), delay_ms);
    }

    fn schedule_at(&self, now: Instant, delay_ms: i64) {
        let delay = Duration::from_millis(delay_ms.max(0) as u64);
        // CEF's requested deadline is authoritative. `i64::MAX` is larger
        // than the range representable by some platform `Instant`s, so keep
        // an unrepresentable deadline safely in the future rather than
        // panicking while converting it.
        let deadline = now.checked_add(delay).unwrap_or_else(|| {
            now.checked_add(Duration::from_secs(365 * 24 * 60 * 60 * 100))
                .expect("a platform Instant must represent a century")
        });
        self.lock().deadline = Some(deadline);
    }

    /// Execute at most one due iteration on the activating thread. Requests
    /// arriving during the iteration survive for a later host-loop tick.
    pub fn run_due(&self, work: impl FnOnce()) -> bool {
        self.run_due_at(Instant::now(), work)
    }

    fn run_due_at(&self, now: Instant, work: impl FnOnce()) -> bool {
        {
            let mut state = self.lock();
            if state.owner != Some(thread::current().id()) {
                return false;
            }
            if state.running {
                state.deadline = Some(now);
                return false;
            }
            if !state.deadline.is_some_and(|deadline| deadline <= now) {
                return false;
            }
            state.deadline = None;
            state.running = true;
        }

        let _iteration = PumpIteration(self);
        work();
        true
    }
}

impl Default for MessagePumpScheduler {
    fn default() -> Self {
        Self::new()
    }
}

struct PumpIteration<'a>(&'a MessagePumpScheduler);

impl Drop for PumpIteration<'_> {
    fn drop(&mut self) {
        let mut state = self.0.lock();
        state.running = false;
        if state.owner.is_some() && state.deadline.is_none() {
            state.deadline = Some(Instant::now() + MAX_IDLE_DELAY);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn remains_inactive_until_initialization_succeeds() {
        let scheduler = MessagePumpScheduler::new();
        scheduler.schedule(0);
        assert!(!scheduler.run_due(|| unreachable!()));
        scheduler.activate();
        assert!(scheduler.run_due(|| {}));
    }

    #[test]
    fn a_new_delayed_request_replaces_the_pending_deadline() {
        let scheduler = MessagePumpScheduler::new();
        scheduler.activate();
        let now = Instant::now();
        scheduler.schedule_at(now, 5);
        scheduler.schedule_at(now, 20);
        assert!(!scheduler.run_due_at(now + Duration::from_millis(5), || unreachable!()));
        assert!(scheduler.run_due_at(now + Duration::from_millis(20), || {}));
    }

    #[test]
    fn immediate_work_advances_a_delayed_request() {
        let scheduler = MessagePumpScheduler::new();
        scheduler.activate();
        let now = Instant::now();
        scheduler.schedule_at(now, 20);
        scheduler.schedule_at(now, -1);
        assert!(scheduler.run_due_at(now, || {}));
    }

    #[test]
    fn another_browser_does_not_reset_the_existing_schedule() {
        let scheduler = MessagePumpScheduler::new();
        scheduler.activate();
        let now = Instant::now();
        scheduler.schedule_at(now, 20);
        scheduler.activate();
        assert!(!scheduler.run_due_at(now, || unreachable!()));
        assert!(scheduler.run_due_at(now + Duration::from_millis(20), || {}));
    }

    #[test]
    fn scheduling_from_another_thread_never_executes_work_there() {
        let scheduler = Arc::new(MessagePumpScheduler::new());
        scheduler.activate();
        let worker_scheduler = Arc::clone(&scheduler);
        let worker = thread::spawn(move || {
            worker_scheduler.schedule(0);
            assert!(!worker_scheduler.run_due(|| unreachable!()));
        });
        assert!(worker.join().is_ok());
        assert!(scheduler.run_due(|| {}));
    }

    #[test]
    fn nested_attempt_is_deferred_and_work_is_never_reentered() {
        let scheduler = MessagePumpScheduler::new();
        scheduler.activate();
        let runs = AtomicUsize::new(0);
        assert!(scheduler.run_due(|| {
            runs.fetch_add(1, Ordering::Relaxed);
            assert!(!scheduler.run_due(|| unreachable!()));
        }));
        assert!(scheduler.run_due(|| {
            runs.fetch_add(1, Ordering::Relaxed);
        }));
        assert_eq!(runs.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn a_request_during_work_is_preserved_without_holding_the_lock() {
        let scheduler = Arc::new(MessagePumpScheduler::new());
        scheduler.activate();
        assert!(scheduler.run_due(|| {
            let worker_scheduler = Arc::clone(&scheduler);
            let worker = thread::spawn(move || worker_scheduler.schedule(0));
            assert!(worker.join().is_ok());
        }));
        assert!(scheduler.run_due(|| {}));
    }

    #[test]
    fn long_cef_deadlines_are_not_clamped() {
        let scheduler = MessagePumpScheduler::new();
        scheduler.activate();
        let now = Instant::now();
        scheduler.schedule_at(now, 100);
        assert!(!scheduler.run_due_at(now, || unreachable!()));
        assert!(!scheduler.run_due_at(now + MAX_IDLE_DELAY, || unreachable!()));
        assert!(scheduler.run_due_at(now + Duration::from_millis(100), || {}));
        assert!(scheduler.lock().deadline.is_some());
    }

    #[test]
    fn an_unrepresentable_cef_deadline_does_not_panic_or_run_early() {
        let scheduler = MessagePumpScheduler::new();
        scheduler.activate();
        let now = Instant::now();
        scheduler.schedule_at(now, i64::MAX);
        assert!(!scheduler.run_due_at(now + Duration::from_secs(1), || unreachable!()));
    }

    #[test]
    fn deactivation_stops_a_scheduled_or_running_pump() {
        let scheduler = MessagePumpScheduler::new();
        scheduler.activate();
        assert!(scheduler.run_due(|| scheduler.deactivate()));
        scheduler.schedule(0);
        assert!(!scheduler.run_due(|| unreachable!()));
    }
}
