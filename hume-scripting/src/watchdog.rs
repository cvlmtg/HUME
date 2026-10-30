use std::cell::Cell;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use hume_core::OsWait;
use steel::steel_vm::ThreadStateController;

/// Enforces a budget for Steel evals: the time spent running Scheme, with
/// waits a builtin reports through [`BudgetClock`], and grammar library
/// opens through the [`OsWait`] it is connected to, left out.
///
/// One watchdog thread persists for the lifetime of the `ScriptingHost` and is
/// re-armed per eval: command dispatch runs on the keystroke hot path, so
/// spawning and joining an OS thread per eval would be pure overhead.
///
/// When an armed budget expires the VM is interrupted through its
/// `ThreadStateController`: from then on every instruction it runs raises,
/// so the script stops whatever it is doing, even inside a `with-handler`
/// that catches everything.
///
/// The armed wait loops on `recv_timeout`, re-checking the deadline on every
/// wake, so an early or spurious wake can never fire the interrupt before the
/// budget has actually elapsed.
///
/// [`EvalWatchdog::cancel`] is synchronous: it blocks until the thread
/// acknowledges it is disarmed and answers whether the budget expired, so no
/// late fire from the previous arm can leak into the next eval.
pub struct EvalWatchdog {
    /// The only strong handle: an [`OsWait`] holds a weak one, so it cannot
    /// keep a dropped watchdog's channel open.
    tx: Arc<Sender<WatchdogMsg>>,
    ack_rx: Receiver<bool>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// Caller-side mirror of the thread's armed/idle state, used only to
    /// `debug_assert` that `arm`/`cancel` calls stay strictly paired. Checked
    /// here, on the caller's own thread, rather than in `watchdog_loop`, so
    /// a violation fails at the actual call site instead of unwinding the
    /// detached watchdog thread (which would otherwise surface later as a
    /// misleading "watchdog thread alive" panic on an unrelated `send`).
    armed: Cell<bool>,
}

enum WatchdogMsg {
    Arm {
        deadline: Instant,
        vm: ThreadStateController,
    },
    Cancel,
    /// A wait on the OS started at `at`: stop the budget's clock.
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
    /// signalled by disconnecting, since `Drop` cannot close the sender
    /// before it joins the thread.
    Shutdown,
}

/// Stops an armed budget's clock while a builtin waits on the OS (a child
/// process, a tree delete), so the budget measures the time spent running
/// Scheme. Ignored while nothing is armed.
pub(crate) struct BudgetClock<'a> {
    tx: &'a Sender<WatchdogMsg>,
}

impl BudgetClock<'_> {
    /// Runs `wait` with the clock stopped. The clock restarts when `wait`
    /// returns or unwinds.
    pub(crate) fn waiting<R>(&self, wait: impl FnOnce() -> R) -> R {
        paused(self.tx, wait)
    }
}

/// Runs `wait` with the clock of the watchdog `tx` sends to stopped,
/// restarting it when `wait` returns or unwinds.
fn paused<R>(tx: &Sender<WatchdogMsg>, wait: impl FnOnce() -> R) -> R {
    struct Restart<'a>(&'a Sender<WatchdogMsg>);
    impl Drop for Restart<'_> {
        fn drop(&mut self) {
            // Never panic in drop; a failed send means the watchdog
            // thread is gone.
            let _ = self.0.send(WatchdogMsg::Resume { at: Instant::now() });
        }
    }
    tx.send(WatchdogMsg::Pause { at: Instant::now() })
        .expect("watchdog thread alive while its clock exists");
    let _restart = Restart(tx);
    wait()
}

impl EvalWatchdog {
    /// Spawn the persistent watchdog thread (idle until the first [`Self::arm`]).
    pub fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let (ack_tx, ack_rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || watchdog_loop(rx, ack_tx));
        Self {
            tx: Arc::new(tx),
            ack_rx,
            thread: Some(thread),
            armed: Cell::new(false),
        }
    }

    /// Arm the budget: `vm` is interrupted once `budget` elapses, unless
    /// [`Self::cancel`] arrives first.  Each `arm` must be paired with one
    /// `cancel` after the eval returns.
    pub fn arm(&self, vm: ThreadStateController, budget: Duration) {
        let was_armed = self.armed.replace(true);
        debug_assert!(
            !was_armed,
            "EvalWatchdog::arm called while already armed: missing cancel() after the previous eval"
        );
        self.send(WatchdogMsg::Arm {
            deadline: Instant::now() + budget,
            vm,
        });
    }

    /// Defuse the current arm and wait for the thread to acknowledge:
    /// `true` when the budget expired and the VM was interrupted.
    ///
    /// Always called after eval returns, on both success and error paths.
    pub fn cancel(&self) -> bool {
        self.armed.set(false);
        self.send(WatchdogMsg::Cancel);
        self.ack_rx
            .recv()
            .expect("watchdog thread alive while EvalWatchdog exists")
    }

    /// A handle a builtin stops this watchdog's clock with while it waits.
    pub(crate) fn clock(&self) -> BudgetClock<'_> {
        BudgetClock { tx: &self.tx }
    }

    /// Connects [`Self::clock`]'s pause to `wait`, for code below the
    /// scripting layer. Once this watchdog is dropped no budget runs, so
    /// `wait` runs each wait as it is.
    pub(crate) fn connect(&self, wait: &OsWait) {
        let tx = Arc::downgrade(&self.tx);
        wait.connect(move |wait| match tx.upgrade() {
            Some(tx) => paused(&tx, wait),
            None => wait(),
        });
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

fn watchdog_loop(rx: Receiver<WatchdogMsg>, ack_tx: Sender<bool>) {
    // Whether the last arm's budget expired, answered by the next Cancel.
    let mut fired = false;
    loop {
        match rx.recv() {
            // Idle → armed.
            Ok(WatchdogMsg::Arm {
                mut deadline,
                mut vm,
            }) => {
                // The budget left when the current wait began.
                let mut paused: Option<Duration> = None;
                loop {
                    let now = Instant::now();
                    if paused.is_none() && now >= deadline {
                        // Every instruction the VM runs from here on raises,
                        // until the session resumes it.
                        vm.interrupt();
                        fired = true;
                        // Back to idle.  The caller's unconditional cancel
                        // arrives later and is acknowledged from the idle arm below.
                        break;
                    }
                    let next = if paused.is_none() {
                        rx.recv_timeout(deadline - now)
                    } else {
                        rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
                    };
                    match next {
                        Ok(WatchdogMsg::Pause { at }) => {
                            paused.get_or_insert(deadline.saturating_duration_since(at));
                        }
                        Ok(WatchdogMsg::Resume { at }) => {
                            if let Some(left) = paused.take() {
                                deadline = at + left;
                            }
                        }
                        Ok(WatchdogMsg::Cancel) => {
                            let _ = ack_tx.send(false);
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
                        Ok(WatchdogMsg::Arm { deadline: d, vm: v }) => {
                            deadline = d;
                            vm = v;
                            paused = None;
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
                let _ = ack_tx.send(std::mem::take(&mut fired));
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
