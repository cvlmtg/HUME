//! The handle the editor gives Steel for an object it keeps on a script's
//! behalf.

use std::cell::Cell;
use std::rc::Rc;

use steel::rvals::SteelVal;

/// Opaque handle the editor returns to Steel for an object it keeps (an
/// open widget, a tracked position), scoping every later call to the
/// instance that minted it: a late callback naming an instance that is gone
/// names a token nothing holds, and reads as a no-op. Crosses the Steel
/// boundary as an integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HostToken(u64);

impl HostToken {
    /// Names nothing: what a `#f` token argument decodes to. The editor's
    /// mint starts at `1`, so no live object holds it.
    pub const NONE: Self = Self(0);

    /// Wraps a counter value the editor minted, or an integer Steel handed
    /// back. Any value is safe to wrap: a forged one matches at most a live
    /// object of the kind it is handed to, never a different kind.
    pub fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// The token as the integer Steel sees.
    pub fn to_steel(self) -> SteelVal {
        SteelVal::IntV(self.0 as isize)
    }
}

/// What every hold on one tracked position shares: whether a callback kept
/// the position, and whether a hold released it.
#[derive(Debug, Default)]
pub struct TrackedFate {
    kept: Cell<bool>,
    released: Cell<bool>,
}

impl TrackedFate {
    /// Keep the position past the callbacks of the requests holding it.
    pub fn keep(&self) {
        self.kept.set(true);
    }

    /// Whether a hold released the position.
    pub fn is_released(&self) -> bool {
        self.released.get()
    }
}

/// A request's hold on a `#:tracked` position. Dropping it, once the
/// request's callback has run or never will, releases the position unless
/// a callback kept it, so a request that ends on any path lets go.
#[derive(Debug)]
pub struct TrackedHold(Rc<TrackedFate>);

impl TrackedHold {
    /// A hold sharing `fate` with the position and its other holds.
    pub fn new(fate: Rc<TrackedFate>) -> Self {
        Self(fate)
    }
}

impl Drop for TrackedHold {
    fn drop(&mut self) {
        if !self.0.kept.get() {
            self.0.released.set(true);
        }
    }
}
