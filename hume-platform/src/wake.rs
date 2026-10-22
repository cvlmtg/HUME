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
/// without polling. `body` is lent the same `wake`, for one that also wakes
/// after each result it posts.
pub fn spawn_waking<T, F>(name: &str, wake: WakeCallback, body: F) -> io::Result<JoinHandle<T>>
where
    F: FnOnce(&WakeCallback) -> T + Send + 'static,
    T: Send + 'static,
{
    thread::Builder::new().name(name.into()).spawn(move || {
        let wake_on_drop = WakeOnDrop(wake);
        body(&wake_on_drop.0)
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
mod tests;
