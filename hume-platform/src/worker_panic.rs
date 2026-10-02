//! Panics on threads other than the main one.
//!
//! The terminal's panic hook restores the screen and the tty modes for the
//! main thread's panic, which ends the process. Run on a background
//! thread's panic it would do the same while the editor keeps drawing, so
//! [`WorkerPanics::install`] wraps it: the main thread's panics reach it
//! unchanged, every other thread's are queued for the editor to report.

use std::any::Any;
use std::backtrace::{Backtrace, BacktraceStatus};
use std::fmt;
use std::panic::{self, PanicHookInfo};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

/// One panic on a background thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerPanic {
    pub thread: String,
    /// `file:line:column` of the panic, when the runtime knows it.
    pub location: Option<String>,
    pub message: String,
    /// The panicking thread's backtrace, present only when `RUST_BACKTRACE`
    /// (or `RUST_LIB_BACKTRACE`) asks for one. Not part of `Display`, which
    /// stays one line for the message log.
    pub backtrace: Option<String>,
}

impl WorkerPanic {
    fn from_hook_info(info: &PanicHookInfo<'_>) -> Self {
        let backtrace = Backtrace::capture();
        Self {
            thread: thread::current().name().unwrap_or("<unnamed>").to_owned(),
            location: info.location().map(ToString::to_string),
            message: payload_message(info.payload()).to_owned(),
            backtrace: (backtrace.status() == BacktraceStatus::Captured)
                .then(|| backtrace.to_string()),
        }
    }
}

impl fmt::Display for WorkerPanic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "thread '{}' panicked", self.thread)?;
        if let Some(location) = &self.location {
            write!(f, " at {location}")?;
        }
        write!(f, ": {}", self.message)
    }
}

/// The text of a panic payload: the message of `panic!("..")` or
/// `panic!("{x}")`, or a placeholder for any other payload type.
pub fn payload_message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload")
}

#[derive(Default)]
struct Queue {
    panics: Vec<WorkerPanic>,
    /// How many of `panics`, from the front, [`WorkerPanics::take_unreported`]
    /// has already handed out.
    reported: usize,
    /// How many of `panics`, from the front, [`WorkerPanics::take_unprinted`]
    /// has already handed out.
    printed: usize,
}

/// The background-thread panics seen since [`WorkerPanics::install`]. Clones
/// share one queue.
#[derive(Clone, Default)]
pub struct WorkerPanics {
    queue: Arc<Mutex<Queue>>,
}

impl WorkerPanics {
    /// Wrap the current panic hook so it runs only for the calling thread's
    /// panics. A panic on any other thread is queued and `wake` is called,
    /// so a loop blocked waiting for input notices it. Neither the wrapped
    /// hook nor the default hook runs for those, so nothing is written to
    /// the screen.
    ///
    /// Call it on the main thread after the terminal is initialized, so the
    /// hook it wraps is the terminal's.
    pub fn install(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let previous = panic::take_hook();
        let main = thread::current().id();
        let panics = Self::default();
        let sink = panics.clone();
        panic::set_hook(Box::new(move |info| {
            if thread::current().id() == main {
                previous(info);
            } else {
                sink.push(WorkerPanic::from_hook_info(info));
                wake();
            }
        }));
        panics
    }

    /// The panics not yet returned by an earlier call, oldest first.
    pub fn take_unreported(&self) -> Vec<WorkerPanic> {
        let mut queue = self.lock();
        let fresh = queue.panics[queue.reported..].to_vec();
        queue.reported = queue.panics.len();
        fresh
    }

    /// The panics not yet returned by an earlier call, oldest first. Counts
    /// separately from [`WorkerPanics::take_unreported`]: one reader feeds
    /// the message log, the other the terminal.
    pub fn take_unprinted(&self) -> Vec<WorkerPanic> {
        let mut queue = self.lock();
        let fresh = queue.panics[queue.printed..].to_vec();
        queue.printed = queue.panics.len();
        fresh
    }

    /// Every panic seen so far, oldest first.
    #[cfg(any(test, feature = "test-util"))]
    pub fn all(&self) -> Vec<WorkerPanic> {
        self.lock().panics.clone()
    }

    fn push(&self, panic: WorkerPanic) {
        self.lock().panics.push(panic);
    }

    /// Queue `panic` as if a background thread had panicked, without a
    /// panic hook.
    #[cfg(any(test, feature = "test-util"))]
    pub fn record(&self, panic: WorkerPanic) {
        self.push(panic);
    }

    /// A panic while the queue is locked would poison it; the queue is plain
    /// data, so a poisoned lock is still safe to read.
    fn lock(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests;
