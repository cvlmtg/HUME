//! The time the editor goes by.

use std::time::{Instant, SystemTime};

#[cfg(test)]
use std::sync::{Arc, Mutex};
#[cfg(test)]
use std::time::Duration;

/// Where the editor reads the current time: every deadline, grace and age
/// it compares is measured on this clock. It reads the system clock; a test
/// can hold it still (`hold`) and move it forward (`advance`), and every
/// clone follows.
#[derive(Clone, Debug, Default)]
pub(crate) struct Clock {
    #[cfg(test)]
    held: Arc<Mutex<Option<Held>>>,
}

/// The time a held clock shows.
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
struct Held {
    instant: Instant,
    wall: SystemTime,
}

impl Clock {
    /// The current monotonic time.
    pub(crate) fn now(&self) -> Instant {
        #[cfg(test)]
        if let Some(held) = self.held() {
            return held.instant;
        }
        Instant::now()
    }

    /// The current wall-clock time, for ages a user reads in minutes and
    /// hours, which count time the machine spent asleep.
    pub(crate) fn wall_now(&self) -> SystemTime {
        #[cfg(test)]
        if let Some(held) = self.held() {
            return held.wall;
        }
        SystemTime::now()
    }

    /// Stop the clock at the current time.
    ///
    /// # Panics
    /// Panics when the clock is already held.
    #[cfg(test)]
    pub(crate) fn hold(&self) {
        let mut held = self.held.lock().expect("the clock lock is never poisoned");
        assert!(held.is_none(), "the clock is already held");
        *held = Some(Held {
            instant: Instant::now(),
            wall: SystemTime::now(),
        });
    }

    /// Move a held clock forward by `by`.
    ///
    /// # Panics
    /// Panics when the clock is not held.
    #[cfg(test)]
    pub(crate) fn advance(&self, by: Duration) {
        let mut held = self.held.lock().expect("the clock lock is never poisoned");
        let held = held.as_mut().expect("advance a held clock");
        held.instant += by;
        held.wall += by;
    }

    #[cfg(test)]
    fn held(&self) -> Option<Held> {
        *self.held.lock().expect("the clock lock is never poisoned")
    }
}
