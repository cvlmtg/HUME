use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use steel::steel_vm::ThreadStateController;

/// Enforces a budget for Steel evals: the time spent running Scheme, with
/// waits a builtin reports through [`BudgetClock`] left out.
///
/// One watchdog thread persists for the lifetime of the `ScriptingHost` and is
/// re-armed per eval: command dispatch runs on the keystroke hot path, so
/// spawning and joining an OS thread per eval would be pure overhead.
///
/// When an armed budget expires the interrupt flag is set to `true` and the
/// VM is interrupted through its `ThreadStateController`: from then on every
/// instruction it runs raises, so the script stops whatever it is doing,
/// even inside a `with-handler` that catches everything.
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
    tx: Sender<WatchdogMsg>,
    ack_rx: Receiver<()>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// Caller-side mirror of the thread's armed/idle state, used only to
    /// `debug_assert` that `arm`/`cancel` calls stay strictly paired. Checked
    /// here, on the caller's own thread, rather than in `watchdog_loop`, so
    /// a violation fails at the actual call site instead of unwinding the
    /// detached watchdog thread (which would otherwise surface later as a
    /// misleading "watchdog thread alive" panic on an unrelated `send`).
    armed: AtomicBool,
}

enum WatchdogMsg {
    Arm {
        deadline: Instant,
        flag: Arc<AtomicBool>,
        vm: ThreadStateController,
    },
    Cancel,
    /// A builtin started waiting on the OS at `at`: stop the budget's clock.
    /// The sender stamps the time, so a late wake of this thread cannot
    /// move it.
    Pause {
        at: Instant,
    },
    /// The wait ended at `at`: the clock runs again with the budget it had
    /// left.
    Resume {
        at: Instant,
    },
    /// The host is dropping its watchdog: the thread exits. Sent rather than
    /// signalled by disconnecting, since a `BudgetClock` keeps a sender open.
    Shutdown,
}

/// Stops an armed budget's clock while a builtin waits on the OS (a child
/// process, a tree delete), so the budget measures the time spent running
/// Scheme. Ignored while nothing is armed.
#[derive(Clone)]
pub(crate) struct BudgetClock {
    tx: Sender<WatchdogMsg>,
}

impl BudgetClock {
    /// Runs `wait` with the clock stopped. The clock restarts when `wait`
    /// returns or unwinds.
    pub(crate) fn waiting<R>(&self, wait: impl FnOnce() -> R) -> R {
        struct Restart<'a>(&'a BudgetClock);
        impl Drop for Restart<'_> {
            fn drop(&mut self) {
                // The watchdog thread outlives every clock its host lends
                // out; a failed send could only mean it is gone already.
                let _ = self.0.tx.send(WatchdogMsg::Resume { at: Instant::now() });
            }
        }
        self.tx
            .send(WatchdogMsg::Pause { at: Instant::now() })
            .expect("watchdog thread alive while its clock exists");
        let _restart = Restart(self);
        wait()
    }
}

impl EvalWatchdog {
    /// Spawn the persistent watchdog thread (idle until the first [`Self::arm`]).
    pub fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let (ack_tx, ack_rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || watchdog_loop(rx, ack_tx));
        Self {
            tx,
            ack_rx,
            thread: Some(thread),
            armed: AtomicBool::new(false),
        }
    }

    /// Arm the budget: `flag` is set to `true` once `budget` elapses, unless
    /// [`Self::cancel`] arrives first.  Each `arm` must be paired with one
    /// `cancel` after the eval returns.
    pub fn arm(&self, flag: Arc<AtomicBool>, vm: ThreadStateController, budget: Duration) {
        debug_assert!(
            !self.armed.swap(true, Ordering::Relaxed),
            "EvalWatchdog::arm called while already armed: missing cancel() after the previous eval"
        );
        self.send(WatchdogMsg::Arm {
            deadline: Instant::now() + budget,
            flag,
            vm,
        });
    }

    /// Defuse the current arm and wait for the thread to acknowledge.
    ///
    /// Always called after eval returns, on both success and error paths.
    /// If the budget already expired, this drains the pending state so the
    /// next `arm` starts clean.
    pub fn cancel(&self) {
        self.armed.store(false, Ordering::Relaxed);
        self.send(WatchdogMsg::Cancel);
        self.ack_rx
            .recv()
            .expect("watchdog thread alive while EvalWatchdog exists");
    }

    /// A handle a builtin stops this watchdog's clock with while it waits.
    pub(crate) fn clock(&self) -> BudgetClock {
        BudgetClock {
            tx: self.tx.clone(),
        }
    }

    fn send(&self, msg: WatchdogMsg) {
        self.tx
            .send(msg)
            .expect("watchdog thread alive while EvalWatchdog exists");
    }
}

impl Drop for EvalWatchdog {
    fn drop(&mut self) {
        // Ignore a thread that already exited or panicked: never panic
        // inside Drop.
        let _ = self.tx.send(WatchdogMsg::Shutdown);
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
                mut vm,
            }) => {
                // Nested waits stop the clock once; the budget left when the
                // outermost began is what the clock restarts with.
                let mut waits = 0u32;
                let mut left = Duration::ZERO;
                loop {
                    let now = Instant::now();
                    if waits == 0 && now >= deadline {
                        flag.store(true, Ordering::Relaxed);
                        // Every instruction the VM runs from here on raises,
                        // until the session resumes it.
                        vm.interrupt();
                        // Fired; back to idle.  The caller's unconditional cancel
                        // arrives later and is acknowledged from the idle arm below.
                        break;
                    }
                    let next = if waits == 0 {
                        rx.recv_timeout(deadline - now)
                    } else {
                        rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
                    };
                    match next {
                        Ok(WatchdogMsg::Pause { at }) => {
                            if waits == 0 {
                                left = deadline.saturating_duration_since(at);
                            }
                            waits += 1;
                        }
                        Ok(WatchdogMsg::Resume { at }) => {
                            waits = waits.saturating_sub(1);
                            if waits == 0 {
                                deadline = at + left;
                            }
                        }
                        Ok(WatchdogMsg::Cancel) => {
                            let _ = ack_tx.send(());
                            break;
                        }
                        // A second Arm while armed cannot happen (arm/cancel are
                        // strictly paired by run_steel_session; `EvalWatchdog::arm`
                        // debug_asserts this on the caller's own thread), but
                        // re-arming is the sensible recovery if it ever does. Never
                        // panic here: this thread is detached, so a panic wouldn't
                        // surface at the call site; it would instead unwind this
                        // thread and turn the *next* unrelated `send` into a
                        // misleading "watchdog thread alive" panic.
                        Ok(WatchdogMsg::Arm {
                            deadline: d,
                            flag: f,
                            vm: v,
                        }) => {
                            deadline = d;
                            flag = f;
                            vm = v;
                            waits = 0;
                        }
                        Ok(WatchdogMsg::Shutdown) => return,
                        // Woke early: the outer loop re-checks the deadline.
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            }
            // Cancel while idle: the previous arm already fired (or there was
            // nothing armed), so acknowledge so the caller's cancel() unblocks.
            Ok(WatchdogMsg::Cancel) => {
                let _ = ack_tx.send(());
            }
            // A wait with no budget armed, or one that outlived its budget's
            // expiry: there is no clock to stop.
            Ok(WatchdogMsg::Pause { .. } | WatchdogMsg::Resume { .. }) => {}
            Ok(WatchdogMsg::Shutdown) | Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests;
