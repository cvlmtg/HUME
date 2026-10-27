//! How a token for an object the editor keeps crosses to Steel, and a
//! request's hold on a tracked position.

use std::cell::Cell;
use std::rc::Rc;

use hume_core::HostToken;
use steel::rvals::SteelVal;

/// `token` as the integer Steel sees.
pub fn token_to_steel(token: HostToken) -> SteelVal {
    SteelVal::IntV(token.raw() as isize)
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
