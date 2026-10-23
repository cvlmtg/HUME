//! A background thread that reports a result as it grows. Each report is
//! the whole result so far, so a consumer that falls behind reads only the
//! latest one and misses nothing.

use std::io;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Weak};

use crate::wake::{WakeCallback, spawn_waking};

/// One read of a producer: its latest snapshot, and whether the producer
/// has ended, so that no later snapshot follows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress<T> {
    pub value: T,
    pub done: bool,
}

/// The producer's side, lent to the body [`spawn_snapshots`] runs.
pub struct Reporter<T> {
    tx: Sender<T>,
    wake: WakeCallback,
    consumer: Weak<()>,
}

impl<T> Reporter<T> {
    /// Reports `value`, the whole result so far, and wakes the consumer.
    /// A report to a consumer that is gone is dropped.
    pub fn send(&self, value: T) {
        if self.tx.send(value).is_ok() {
            (self.wake)();
        }
    }

    /// Whether the consumer still wants reports. The body checks it before
    /// each piece of work and stops once it is `false`.
    pub fn is_live(&self) -> bool {
        self.consumer.strong_count() > 0
    }
}

/// The consumer's side of a producer [`spawn_snapshots`] started. Dropping
/// it tells the producer to stop, at its next [`Reporter::is_live`] check.
pub struct Snapshots<T> {
    rx: Receiver<T>,
    latest: T,
    ended: bool,
    /// The only strong reference to what each [`Reporter`] holds weakly.
    _alive: Arc<()>,
}

impl<T: Clone> Snapshots<T> {
    /// The latest snapshot when there is news since the last call: a newer
    /// snapshot, or the producer's end. A producer that returns or panics
    /// ends with its last snapshot, or `T::default()` if it reported none.
    /// `None` without news, and after the end has been read.
    pub fn try_take(&mut self) -> Option<Progress<T>> {
        if self.ended {
            return None;
        }
        let mut fresh = false;
        loop {
            match self.rx.try_recv() {
                Ok(value) => {
                    self.latest = value;
                    fresh = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.ended = true;
                    break;
                }
            }
        }
        (fresh || self.ended).then(|| Progress {
            value: self.latest.clone(),
            done: self.ended,
        })
    }
}

/// Runs `body` on a thread named `name`. Each report `body` makes through
/// the [`Reporter`] it is lent calls `wake`, and so does the thread's end.
pub fn spawn_snapshots<T, F>(name: &str, wake: WakeCallback, body: F) -> io::Result<Snapshots<T>>
where
    T: Default + Send + 'static,
    F: FnOnce(&Reporter<T>) + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    let alive = Arc::new(());
    let consumer = Arc::downgrade(&alive);
    // The reporter, and with it the sender, drops before `spawn_waking`'s
    // end-of-thread wake, so a drain woken by it sees the end.
    spawn_waking(name, wake, move |wake| {
        let reporter = Reporter {
            tx,
            wake: Arc::clone(wake),
            consumer,
        };
        body(&reporter);
    })?;
    Ok(Snapshots {
        rx,
        latest: T::default(),
        ended: false,
        _alive: alive,
    })
}

#[cfg(test)]
mod tests;
