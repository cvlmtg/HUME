use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

/// Enforces a wall-clock budget for Steel evals.
///
/// One watchdog thread persists for the lifetime of the `ScriptingHost` and is
/// re-armed per eval — command dispatch runs on the keystroke hot path, so
/// spawning and joining an OS thread per eval would be pure overhead.
///
/// When an armed budget expires the interrupt flag is set to `true`,
/// signalling `(hume/yield!)` calls inside the script to abort.  Interruption
/// is cooperative only — Steel 0.8.2 has no op-callback for involuntary stop.
///
/// The armed wait loops on `recv_timeout`, re-checking the deadline on every
/// wake, so an early or spurious wake can never fire the interrupt before the
/// budget has actually elapsed.
///
/// [`EvalWatchdog::cancel`] is synchronous: it blocks until the thread
/// acknowledges it is disarmed.  The caller resets the interrupt flag right
/// after cancelling, and the ack guarantees no late fire from the previous
/// arm can leak into the next eval.
pub struct EvalWatchdog {
    /// `None` only during `Drop`, where it is taken to disconnect the channel
    /// so the thread's `recv` unblocks and the loop exits.
    tx: Option<Sender<WatchdogMsg>>,
    ack_rx: Receiver<()>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// Caller-side mirror of the thread's armed/idle state, used only to
    /// `debug_assert` that `arm`/`cancel` calls stay strictly paired. Checked
    /// here — on the caller's own thread — rather than in `watchdog_loop`, so
    /// a violation fails at the actual call site instead of unwinding the
    /// detached watchdog thread (which would otherwise surface later as a
    /// misleading "watchdog thread alive" panic on an unrelated `send`).
    armed: AtomicBool,
}

enum WatchdogMsg {
    Arm {
        deadline: Instant,
        flag: Arc<AtomicBool>,
    },
    Cancel,
}

impl EvalWatchdog {
    /// Spawn the persistent watchdog thread (idle until the first [`Self::arm`]).
    pub fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let (ack_tx, ack_rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || watchdog_loop(rx, ack_tx));
        Self {
            tx: Some(tx),
            ack_rx,
            thread: Some(thread),
            armed: AtomicBool::new(false),
        }
    }

    /// Arm the budget: `flag` is set to `true` once `budget` elapses, unless
    /// [`Self::cancel`] arrives first.  Each `arm` must be paired with one
    /// `cancel` after the eval returns.
    pub fn arm(&self, flag: Arc<AtomicBool>, budget: Duration) {
        debug_assert!(
            !self.armed.swap(true, Ordering::Relaxed),
            "EvalWatchdog::arm called while already armed — missing cancel() after the previous eval"
        );
        self.send(WatchdogMsg::Arm {
            deadline: Instant::now() + budget,
            flag,
        });
    }

    /// Defuse the current arm and wait for the thread to acknowledge.
    ///
    /// Always called after eval returns — on both success and error paths.
    /// If the budget already expired, this drains the pending state so the
    /// next `arm` starts clean.
    pub fn cancel(&self) {
        self.armed.store(false, Ordering::Relaxed);
        self.send(WatchdogMsg::Cancel);
        self.ack_rx
            .recv()
            .expect("watchdog thread alive while EvalWatchdog exists");
    }

    fn send(&self, msg: WatchdogMsg) {
        self.tx
            .as_ref()
            .expect("tx is Some until Drop")
            .send(msg)
            .expect("watchdog thread alive while EvalWatchdog exists");
    }
}

impl Drop for EvalWatchdog {
    fn drop(&mut self) {
        // Disconnect the channel so the thread's blocking recv returns Err
        // and the loop exits; then join.  Ignore a panicked thread — never
        // panic inside Drop.
        self.tx = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Default for EvalWatchdog {
    fn default() -> Self {
        Self::new()
    }
}

fn watchdog_loop(rx: Receiver<WatchdogMsg>, ack_tx: Sender<()>) {
    loop {
        match rx.recv() {
            // Idle → armed.
            Ok(WatchdogMsg::Arm {
                mut deadline,
                mut flag,
            }) => loop {
                let now = Instant::now();
                if now >= deadline {
                    flag.store(true, Ordering::Relaxed);
                    // Fired; back to idle.  The caller's unconditional cancel
                    // arrives later and is acknowledged from the idle arm below.
                    break;
                }
                match rx.recv_timeout(deadline - now) {
                    Ok(WatchdogMsg::Cancel) => {
                        let _ = ack_tx.send(());
                        break;
                    }
                    // A second Arm while armed cannot happen (arm/cancel are
                    // strictly paired by run_steel_session; `EvalWatchdog::arm`
                    // debug_asserts this on the caller's own thread), but
                    // re-arming is the sensible recovery if it ever does. Never
                    // panic here: this thread is detached, so a panic wouldn't
                    // surface at the call site — it would instead unwind this
                    // thread and turn the *next* unrelated `send` into a
                    // misleading "watchdog thread alive" panic.
                    Ok(WatchdogMsg::Arm {
                        deadline: d,
                        flag: f,
                    }) => {
                        deadline = d;
                        flag = f;
                    }
                    // Woke early — the outer loop re-checks the deadline.
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            },
            // Cancel while idle: the previous arm already fired (or there was
            // nothing armed) — acknowledge so the caller's cancel() unblocks.
            Ok(WatchdogMsg::Cancel) => {
                let _ = ack_tx.send(());
            }
            // Sender dropped (host shutdown) — exit.
            Err(_) => return,
        }
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
        let dog = EvalWatchdog::new();
        dog.arm(Arc::clone(&flag), Duration::from_millis(20));
        // Sleep 10× the budget to rule out timing races on slow CI.
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            flag.load(Ordering::Relaxed),
            "flag must be set after budget expires"
        );
        dog.cancel(); // cancel-after-expiry must ack from idle, not hang
    }

    #[test]
    fn cancel_prevents_flag() {
        let flag = Arc::new(AtomicBool::new(false));
        let dog = EvalWatchdog::new();
        // Use a 30-second budget — the watchdog must not fire before cancel().
        dog.arm(Arc::clone(&flag), Duration::from_secs(30));
        dog.cancel(); // synchronous: the thread is disarmed when this returns
        assert!(
            !flag.load(Ordering::Relaxed),
            "flag must remain false after cancel"
        );
    }

    /// An unpaired second `arm` (no `cancel` in between) must be caught by the
    /// caller-side `debug_assert` in `arm` itself — on the test's own thread —
    /// rather than silently recovering inside the detached watchdog thread.
    ///
    /// Fail oracle: move the assert into `watchdog_loop` (checked on the
    /// detached thread) → this test's `#[should_panic]` never fires because
    /// nothing on the *caller's* stack panics.
    #[test]
    #[should_panic(expected = "missing cancel() after the previous eval")]
    #[cfg(debug_assertions)]
    fn unpaired_arm_is_caught_on_callers_thread() {
        let flag = Arc::new(AtomicBool::new(false));
        let dog = EvalWatchdog::new();
        dog.arm(Arc::clone(&flag), Duration::from_secs(30));
        dog.arm(flag, Duration::from_secs(30)); // missing cancel() — must panic here
    }

    /// The persistent thread survives an arm/cancel cycle and fires on a
    /// subsequent arm.
    ///
    /// Fail oracle: make the thread exit after the first cancel → the second
    /// arm never fires (or `arm` panics on a dead channel).
    #[test]
    fn rearming_after_cancel_still_fires() {
        let flag = Arc::new(AtomicBool::new(false));
        let dog = EvalWatchdog::new();
        dog.arm(Arc::clone(&flag), Duration::from_secs(30));
        dog.cancel();
        assert!(!flag.load(Ordering::Relaxed));

        dog.arm(Arc::clone(&flag), Duration::from_millis(20));
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            flag.load(Ordering::Relaxed),
            "second arm must fire after its budget"
        );
        dog.cancel();
    }

    /// Dropping the watchdog while armed shuts the thread down without firing.
    #[test]
    fn drop_while_armed_does_not_fire() {
        let flag = Arc::new(AtomicBool::new(false));
        {
            let dog = EvalWatchdog::new();
            dog.arm(Arc::clone(&flag), Duration::from_secs(30));
            // drop joins the thread; the armed wait sees Disconnected and exits
        }
        assert!(
            !flag.load(Ordering::Relaxed),
            "flag must remain false when dropped before the budget"
        );
    }
}
