//! SIGCHLD as a counter that waiting threads block on.
//!
//! One process-wide thread reads a self-pipe that SIGCHLD writes to and
//! bumps `ExitSignal`'s generation on every read. Several exits can share
//! one signal, and a signal can be for a child nobody waits on, so a bump
//! only tells each waiter to recheck its own child. The thread never touches
//! a child, so no child's lock is held while anyone blocks here.

use std::io::{self, Read};
use std::os::unix::net::UnixStream;
use std::sync::{Condvar, Mutex, OnceLock, PoisonError};

use signal_hook::consts::signal::SIGCHLD;

pub(super) struct ExitSignal {
    generation: Mutex<u64>,
    changed: Condvar,
}

impl ExitSignal {
    /// The number of SIGCHLD reads so far. Read it before checking a child,
    /// then [`wait_past`](Self::wait_past) it: an exit after the read moves
    /// it, so none is missed.
    pub(super) fn generation(&self) -> u64 {
        *self
            .generation
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Block until the generation is past `seen`.
    pub(super) fn wait_past(&self, seen: u64) {
        let generation = self
            .generation
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        drop(
            self.changed
                .wait_while(generation, |now| *now == seen)
                .unwrap_or_else(PoisonError::into_inner),
        );
    }

    fn bump(&self) {
        *self
            .generation
            .lock()
            .unwrap_or_else(PoisonError::into_inner) += 1;
        self.changed.notify_all();
    }
}

static SIGNAL: ExitSignal = ExitSignal {
    generation: Mutex::new(0),
    changed: Condvar::new(),
};

/// The process's exit signal, its watcher started on the first call. `Err`
/// on every call when the watcher could not start.
pub(super) fn exit_signal() -> io::Result<&'static ExitSignal> {
    static WATCHER: OnceLock<Result<(), String>> = OnceLock::new();
    match WATCHER.get_or_init(|| start_watcher().map_err(|e| e.to_string())) {
        Ok(()) => Ok(&SIGNAL),
        Err(e) => Err(io::Error::other(format!(
            "cannot watch for child processes exiting: {e}"
        ))),
    }
}

fn start_watcher() -> io::Result<()> {
    let (mut read, write) = UnixStream::pair()?;
    signal_hook::low_level::pipe::register(SIGCHLD, write)?;
    std::thread::Builder::new()
        .name("hume-sigchld".into())
        .spawn(move || {
            let mut buf = [0u8; 64];
            loop {
                match read.read(&mut buf) {
                    Ok(0) => panic!("the SIGCHLD registration closed its pipe"),
                    Ok(_) => SIGNAL.bump(),
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => panic!("reading the SIGCHLD pipe failed: {e}"),
                }
            }
        })?;
    Ok(())
}
