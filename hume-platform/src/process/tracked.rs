//! Process-wide tracking of long-lived children, so a force-exit can still
//! reap them.
//!
//! `std::process::exit` (`crate::force_exit`'s last step) runs no
//! destructors — the `Drop` impls that normally kill `hume-lsp`'s LSP
//! servers and the picker's line-source children (`spawn_line_source`)
//! never fire on a signal/hangup force-exit. Every long-lived child
//! registers itself here via [`TrackedChild::new`] at spawn time;
//! [`kill_tracked_children`] is called from `force_exit` immediately before
//! tearing the terminal down and exiting, as a fail-safe alongside each
//! type's own `Drop` (which still runs, and still owns cleanup, on every
//! normal exit).
//!
//! A process-global table is the only way to reach this from `force_exit`:
//! the terminator thread's closure captures only a `SharedTerm` and a
//! `request_quit` callback, with no structural path to any `Child` spawned
//! elsewhere in the process.
//!
//! Scope: long-lived children only. A synchronous `.status()` child
//! (`tree_sitter_build`, `unpack_zip`/`unpack_gz`, `run_inline_output`) runs
//! on the main thread and is never registered here, so a force-exit mid-call
//! can orphan it. Accepted rather than fixed — those children are
//! short-lived, self-terminating, and their output is already streaming to
//! the user's terminal, and tracking them would need a blocking `wait` held
//! under the same slot mutex `kill_slot`'s retry bound exists to keep short.

use std::process::Child;
use std::sync::{Arc, Mutex, TryLockError, Weak};
use std::time::Duration;

#[cfg(unix)]
use nix::sys::signal::{Signal, killpg};
#[cfg(unix)]
use nix::unistd::{Pid, getpgid};

struct TrackedInner {
    child: Child,
    /// Recorded once at registration rather than re-checked at kill time:
    /// by the time `force_exit` runs the child may already be gone, and
    /// `getpgid` on a dead pid would just report `ESRCH` — this remembers
    /// what `spawn_in_own_group` actually established, not what's still
    /// true this instant.
    #[cfg_attr(not(unix), allow(dead_code))]
    group_leader: bool,
}

impl TrackedInner {
    /// Kill, group-directed when this child leads its own process group so
    /// grandchildren (rust-analyzer's `proc-macro-srv`, build scripts, ...)
    /// go down with it — shared by every kill path (a normal-exit `Drop`'s
    /// [`TrackedChild::reap`] and [`kill_tracked_children`]'s force-exit
    /// reap) so a grandchild is never left behind depending on which one
    /// happens to run.
    fn kill(&mut self) {
        // `Child::kill`'s own pid-reuse guard (checked in the vendored `std`
        // source: it no-ops once the child's exit status has been cached)
        // does not cover the `killpg` path below, which bypasses `Child`
        // entirely and signals a raw pid via `nix`. Consult (and cache) the
        // exit status the same way `Child::kill` does internally, so a
        // child that already exited — and whose pid the kernel may already
        // have reused — is never signalled at all, group or otherwise.
        if !matches!(self.child.try_wait(), Ok(None)) {
            return;
        }
        #[cfg(unix)]
        if self.group_leader
            && let Ok(pid) = i32::try_from(self.child.id())
            // Group-directed: reaches grandchildren (rust-analyzer's
            // `proc-macro-srv`, build scripts, ...) that signalling just the
            // tracked pid would leave orphaned. Falls through to the direct
            // kill below on failure — an unconditional `return` here would
            // leave the child unsignalled and `reap()`'s `wait()` blocking
            // forever on it.
            && killpg(Pid::from_raw(pid), Signal::SIGKILL).is_ok()
        {
            return;
        }
        let _ = self.child.kill();
    }
}

type ChildSlot = Mutex<TrackedInner>;

/// A table of live registrations, held as `Weak` so a consumer's own
/// `Drop`/reap — the common case, on every normal exit — needs no explicit
/// deregistration call; an entry is simply unreachable once nothing
/// strong-refs it.
///
/// A value type, not a bare `static`, so tests can hold a private instance:
/// a shared global would make [`kill_all`](Self::kill_all) in one test kill
/// children a *different*, concurrently-running test just spawned —
/// `cargo test` runs the whole crate's tests as threads in one process, and
/// nothing here scopes a table to a single test.
struct ChildRegistry {
    entries: Mutex<Vec<Weak<ChildSlot>>>,
}

impl ChildRegistry {
    const fn new() -> Self {
        ChildRegistry {
            entries: Mutex::new(Vec::new()),
        }
    }

    fn register(&self, slot: &Arc<ChildSlot>) {
        let mut table = lock_recovering(&self.entries);
        table.retain(|w| w.strong_count() > 0);
        table.push(Arc::downgrade(slot));
    }

    /// Kills every still-registered child, best-effort. Must never block the
    /// exit this exists to make possible on a *slot* — each is a `try_lock`,
    /// and a slot already held (a concurrent normal-shutdown `Drop` reaping
    /// the same child) is skipped rather than waited on, not fought over.
    /// The table lock itself is a plain blocking `lock`: it only ever guards
    /// a `Vec` retain + push in [`register`](Self::register), never a
    /// syscall, so waiting for it is bounded and cheap — unlike skipping it,
    /// which would abandon every child rather than just one.
    fn kill_all(&self) {
        let table = lock_recovering(&self.entries);
        for weak in table.iter() {
            if let Some(slot) = weak.upgrade() {
                kill_slot(&slot);
            }
        }
    }

    #[cfg(all(test, unix))]
    fn live_count(&self) -> usize {
        lock_recovering(&self.entries)
            .iter()
            .filter(|w| w.strong_count() > 0)
            .count()
    }
}

static GLOBAL: ChildRegistry = ChildRegistry::new();

/// Kills every child still registered in the process-wide table. Called
/// from `force_exit`, immediately before tearing the terminal down and
/// exiting.
pub(crate) fn kill_tracked_children() {
    GLOBAL.kill_all();
}

/// A spawned child registered with the process-wide force-exit reaper.
///
/// Not a replacement for a type's own kill/wait `Drop` — `ServerHandle` and
/// `SpawnedLineSource` still do that themselves on every normal exit. This
/// only exists to cover the abnormal case where nothing gets to run a `Drop`
/// at all. `Child::kill`/`wait` cache the reaped exit status and turn a
/// repeat call into a no-op (verified in the vendored `std` source), which
/// is what makes it safe for both a normal-exit `Drop` and this table's own
/// reap to call [`reap`](Self::reap) without coordinating who goes first.
#[derive(Clone)]
pub struct TrackedChild(Arc<ChildSlot>);

impl TrackedChild {
    /// Registers `child` with the process-wide reaper. Pair with
    /// [`spawn_in_own_group`](super::spawn_in_own_group) so the
    /// group-leader check below actually has a group to find.
    pub fn new(child: Child) -> Self {
        Self::in_registry(child, &GLOBAL)
    }

    fn in_registry(child: Child, registry: &ChildRegistry) -> Self {
        #[cfg(unix)]
        let group_leader = i32::try_from(child.id())
            .map(Pid::from_raw)
            .is_ok_and(|pid| getpgid(Some(pid)) == Ok(pid));
        #[cfg(not(unix))]
        let group_leader = false;

        let slot = Arc::new(Mutex::new(TrackedInner {
            child,
            group_leader,
        }));
        registry.register(&slot);
        TrackedChild(slot)
    }

    /// The child's OS process id.
    pub fn id(&self) -> u32 {
        lock_recovering(&self.0).child.id()
    }

    /// Non-blocking wait — does not kill. Mirrors `Child::try_wait`; callers
    /// that need the "still running → kill it" fallback call
    /// [`reap`](Self::reap) themselves on `Ok(None)`.
    pub fn try_wait(&self) -> std::io::Result<Option<std::process::ExitStatus>> {
        lock_recovering(&self.0).child.try_wait()
    }

    /// Kill (group-directed when this child leads its own process group —
    /// see [`TrackedInner::kill`]) + wait. Safe to call more than once,
    /// including racing a concurrent [`kill_tracked_children`] — see the
    /// type doc.
    pub fn reap(&self) -> Option<std::process::ExitStatus> {
        let mut inner = lock_recovering(&self.0);
        inner.kill();
        inner.child.wait().ok()
    }
}

/// Attempts before a contended slot is abandoned. A slot held by a
/// concurrent [`TrackedChild::reap`] needs no help — that path kills the
/// child itself — but [`TrackedChild::id`]/[`TrackedChild::try_wait`] hold
/// the same mutex without killing anything, so one failed `try_lock` is not
/// proof the child is being dealt with. Bounded: force-exit must never wait
/// indefinitely on a slot.
const KILL_LOCK_ATTEMPTS: u32 = 5;

/// Delay between [`KILL_LOCK_ATTEMPTS`] retries.
const KILL_LOCK_RETRY: Duration = Duration::from_millis(1);

fn kill_slot(slot: &Arc<ChildSlot>) {
    for attempt in 0..KILL_LOCK_ATTEMPTS {
        match try_lock_recovering(slot) {
            Some(mut inner) => {
                inner.kill();
                return;
            }
            None if attempt + 1 < KILL_LOCK_ATTEMPTS => {
                std::thread::sleep(KILL_LOCK_RETRY);
            }
            None => {}
        }
    }
}

fn lock_recovering<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn try_lock_recovering<T>(m: &Mutex<T>) -> Option<std::sync::MutexGuard<'_, T>> {
    match m.try_lock() {
        Ok(guard) => Some(guard),
        Err(TryLockError::Poisoned(e)) => Some(e.into_inner()),
        Err(TryLockError::WouldBlock) => None,
    }
}

#[cfg(test)]
mod tests;
