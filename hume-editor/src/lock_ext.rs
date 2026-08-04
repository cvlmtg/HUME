use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Poisoning is not part of this crate's error model — every `RwLock` here
/// guards frame-local render/decoration state with no held-lock panics, so a
/// poisoned lock would itself be a bug worth crashing on, not a recoverable
/// error. This states that assumption once instead of at each call site.
pub(crate) trait LockExt<T> {
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
