use std::sync::{Arc, RwLock, RwLockReadGuard};

/// A shared, frame-local mutable slot — `Arc<RwLock<T>>` with this
/// workspace's poison policy baked in, for the render/overlay/decoration
/// state shared across crate boundaries via a raw `Arc` (`hume-ui`'s
/// overlay views and pane-render handles, `hume-decorations`'s per-buffer
/// stores): none of it is held across a panic, so a poisoned lock would
/// itself be a bug worth crashing on, not a recoverable error. Lives here,
/// not in either of those crates, so both share one implementation and one
/// policy instead of each re-deriving it.
///
/// `read()`/`set()` are the whole API: every real write site in both
/// crates replaces the entire value rather than mutating it in place (the
/// one exception, a bracket-match highlight's clear-then-extend, replaces
/// with a freshly built `Vec` instead), so there is no caller that needs a
/// write guard. `Clone` bumps the refcount, matching every registration
/// site's former `Arc::clone(&x)`.
pub struct SharedSlot<T>(Arc<RwLock<T>>);

impl<T> SharedSlot<T> {
    pub fn new(value: T) -> Self {
        Self(Arc::new(RwLock::new(value)))
    }

    pub fn read(&self) -> RwLockReadGuard<'_, T> {
        self.0.read().expect("RwLock not poisoned")
    }

    pub fn set(&self, value: T) {
        *self.0.write().expect("RwLock not poisoned") = value;
    }
}

impl<T> Clone for SharedSlot<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T: Default> Default for SharedSlot<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}
