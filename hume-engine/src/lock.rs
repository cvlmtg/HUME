use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Poisoning is not part of this workspace's error model for the
/// frame-local render/overlay/decoration state shared via `Arc<RwLock<_>>`
/// (`hume-ui`'s overlay views and pane-render handles, `hume-decorations`'
/// per-buffer stores) — none of it is held across a panic, so a poisoned
/// lock would itself be a bug worth crashing on, not a recoverable error.
/// Lives here, not in either of those crates, so both can share one
/// implementation instead of each re-deriving the same policy.
pub trait LockExt<T> {
    fn read_or_panic(&self) -> RwLockReadGuard<'_, T>;
    fn write_or_panic(&self) -> RwLockWriteGuard<'_, T>;
}

impl<T> LockExt<T> for RwLock<T> {
    fn read_or_panic(&self) -> RwLockReadGuard<'_, T> {
        self.read().expect("RwLock not poisoned")
    }

    fn write_or_panic(&self) -> RwLockWriteGuard<'_, T> {
        self.write().expect("RwLock not poisoned")
    }
}
