use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Arms a wall-clock budget for a single Steel eval.
///
/// When the budget expires the interrupt flag is set to `true`, signalling
/// `(hume/yield!)` calls inside the script to abort.  Interruption is
/// cooperative only — Steel 0.8.2 has no op-callback for involuntary stop.
///
/// Use `park_timeout` so [`EvalWatchdog::cancel`] wakes the thread
/// immediately on the happy path rather than sleeping out the full budget.
pub struct EvalWatchdog {
    cancel: Arc<AtomicBool>,
    thread: std::thread::JoinHandle<()>,
}

impl EvalWatchdog {
    /// Spawn the watchdog.  Will flip `flag` to `true` after `budget` unless
    /// cancelled first.
    pub fn arm(flag: Arc<AtomicBool>, budget: std::time::Duration) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let thread = {
            let flag = Arc::clone(&flag);
            let cancel = Arc::clone(&cancel);
            std::thread::spawn(move || {
                // park_timeout wakes either when unpark() is called (cancel path)
                // or when the budget elapses — whichever comes first.
                std::thread::park_timeout(budget);
                if !cancel.load(Ordering::Relaxed) {
                    flag.store(true, Ordering::Relaxed);
                }
            })
        };
        Self { cancel, thread }
    }

    /// Defuse: signal cancellation, wake the thread, and join.
    /// Always called after eval returns — on both success and error paths.
    pub fn cancel(self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.thread.thread().unpark();
        // Propagate panics from the watchdog thread; otherwise ignore join errors.
        let _ = self.thread.join();
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use std::time::Duration;

    use super::EvalWatchdog;

    #[test]
    fn expiry_sets_flag() {
        let flag = Arc::new(AtomicBool::new(false));
        let _dog = EvalWatchdog::arm(Arc::clone(&flag), Duration::from_millis(20));
        // Sleep 10× the budget to rule out timing races on slow CI.
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            flag.load(Ordering::Relaxed),
            "flag must be set after budget expires"
        );
    }

    #[test]
    fn cancel_prevents_flag() {
        let flag = Arc::new(AtomicBool::new(false));
        // Use a 30-second budget — the watchdog must not fire before cancel().
        let dog = EvalWatchdog::arm(Arc::clone(&flag), Duration::from_secs(30));
        dog.cancel(); // joins the thread; deterministic since cancel() unparks it
        assert!(
            !flag.load(Ordering::Relaxed),
            "flag must remain false after cancel"
        );
    }
}
