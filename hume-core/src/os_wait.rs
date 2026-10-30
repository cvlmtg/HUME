use std::sync::{Arc, Mutex};

/// Runs a call that blocks on the OS with the running script's budget
/// stopped, for code below the scripting layer that a script can reach.
/// Clones share one pause, which each scripting host connects when it
/// starts. Until one does, and once that host is gone, a wait runs as it is.
#[derive(Clone, Default)]
pub struct OsWait(Arc<Mutex<Option<Arc<Pause>>>>);

/// Stops the budget, runs the wait it is given, and restarts the budget.
type Pause = dyn Fn(&mut dyn FnMut()) + Send + Sync;

impl OsWait {
    /// Runs every clone's waits through `pause` from now on, in place of the
    /// pause connected before. `pause` must call the wait it is given once.
    pub fn connect(&self, pause: impl Fn(&mut dyn FnMut()) + Send + Sync + 'static) {
        *self.slot() = Some(Arc::new(pause));
    }

    /// Runs `wait` with the budget stopped.
    pub fn waiting<R>(&self, wait: impl FnOnce() -> R) -> R {
        let Some(pause) = self.slot().clone() else {
            return wait();
        };
        let mut wait = Some(wait);
        let mut result = None;
        pause(&mut || result = wait.take().map(|wait| wait()));
        result.expect("an OsWait's pause calls the wait it is given")
    }

    fn slot(&self) -> std::sync::MutexGuard<'_, Option<Arc<Pause>>> {
        self.0
            .lock()
            .expect("an OsWait's lock is held only to read or replace its pause")
    }
}
