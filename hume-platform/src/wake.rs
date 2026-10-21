//! Waking the editor's main loop from a background thread, so it drains the
//! thread's results instead of polling for them.

use std::io;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

/// Called by a background thread once it has something to hand off.
/// Type-erased so no crate's API names `termina`'s waker; production wraps
/// `termina::PlatformWaker::wake`. Callers tolerate spurious wakes.
pub type WakeCallback = Arc<dyn Fn() + Send + Sync>;

/// Calls its [`WakeCallback`] when dropped, on a normal return and on an
/// unwind alike.
struct WakeOnDrop(WakeCallback);

impl Drop for WakeOnDrop {
    fn drop(&mut self) {
        (self.0)();
    }
}

/// Spawns a thread named `name` running `body`, and calls `wake` once when
/// the thread ends, whether `body` returns or panics. A drain woken that
/// way sees the thread's channel disconnect, so a dead worker is noticed
/// without polling. A `body` that also wakes after each result it posts
/// takes its own clone of `wake`.
pub fn spawn_waking<T, F>(name: &str, wake: WakeCallback, body: F) -> io::Result<JoinHandle<T>>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    thread::Builder::new().name(name.into()).spawn(move || {
        let _wake_on_drop = WakeOnDrop(wake);
        body()
    })
}

/// A wake for a thread whose wakes the test does not wait on.
#[cfg(any(test, feature = "test-util"))]
pub fn no_op_wake() -> WakeCallback {
    Arc::new(|| {})
}

/// A wake that records each call on the returned receiver, so a test can
/// wait for one with `recv_timeout` or count them with `try_iter`.
#[cfg(any(test, feature = "test-util"))]
pub fn counting_wake() -> (WakeCallback, std::sync::mpsc::Receiver<()>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let wake: WakeCallback = Arc::new(move || {
        let _ = tx.send(());
    });
    (wake, rx)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn a_waking_thread_wakes_once_when_its_body_returns() {
        let (wake, woken) = counting_wake();
        let handle = spawn_waking("hume-test-waking", wake, || 7).expect("spawn");
        assert_eq!(handle.join().expect("no panic"), 7);
        assert_eq!(woken.try_iter().count(), 1);
    }

    #[test]
    fn a_waking_thread_wakes_when_its_body_panics() {
        let (wake, woken) = counting_wake();
        let handle = spawn_waking::<(), _>("hume-test-waking", wake, || {
            panic!("simulated worker crash")
        })
        .expect("spawn");
        assert!(handle.join().is_err(), "the body panicked");
        woken
            .recv_timeout(Duration::from_secs(1))
            .expect("the unwind wakes");
    }
}
