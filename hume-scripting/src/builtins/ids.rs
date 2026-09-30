//! Opaque Steel types for buffer/pane identity.
//!
//! [`SteelPane`] is the crossing-boundary value: every command, hook,
//! completion source, and builtin argument or return that names a buffer
//! carries a `PaneHandle` wrapped in `SteelPane`. Plugins receive and pass
//! these between builtins but cannot construct or inspect them
//! arithmetically; they are purely opaque handles. [`SteelBufferKey`] is
//! `(buffer-key pane)`'s own return type: a per-buffer hash/comparison key,
//! undecodable by [`super::args::ArgPane`]/[`super::args::LivePane`]
//! so a key can't be passed back into a builtin expecting a pane. A pane
//! value and a plain per-buffer key must stay two distinct kinds of thing,
//! not the same value with its pane field cleared. Collapsing them would
//! let a key masquerade as a pane-less handle anywhere a pane is expected,
//! silently reintroducing the implicit-pane guessing this design exists to
//! remove (see `types::PaneHandle`'s own doc).
//!
//! Display uses the slotmap `as_ffi` u64 so that `(log! "info" pane)` prints
//! something readable without revealing internal structure.

use hume_engine::pipeline::BufferId;
use slotmap::Key as _;
use steel::{
    gc::ShareableMut as _,
    rvals::{Custom, IntoSteelVal as _, SteelVal, as_underlying_type},
};

use crate::types::PaneHandle;

// ── Wrapper types ─────────────────────────────────────────────────────────────

/// Opaque Steel handle for a [`PaneHandle`]; see this module's own doc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SteelPane(pub(crate) PaneHandle);

impl SteelPane {
    /// Wrap a `PaneHandle` into a Steel-facing opaque handle.
    pub fn new(handle: PaneHandle) -> Self {
        Self(handle)
    }

    /// Convert to a `SteelVal` without returning `Result`.
    ///
    /// `IntoSteelVal` for custom types is infallible; this avoids `.expect()` at
    /// every call site that wraps a `PaneHandle` for hook args or builtin returns.
    pub fn into_steel_val(self) -> SteelVal {
        self.into_steelval().expect("SteelPane into_steelval")
    }
}

/// `(buffer-key pane)`'s return: `pane`'s buffer, with no pane component.
/// See this module's own doc for why this is a distinct type rather than a
/// [`SteelPane`] with its pane field cleared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct SteelBufferKey(pub(crate) BufferId);

impl Custom for SteelPane {
    fn fmt(&self) -> Option<Result<String, std::fmt::Error>> {
        Some(Ok(match self.0.pane() {
            Some(pid) => format!(
                "#<pane buffer={} pane={}>",
                self.0.buffer().data().as_ffi(),
                pid.data().as_ffi()
            ),
            None => format!("#<pane buffer={}>", self.0.buffer().data().as_ffi()),
        }))
    }

    fn equality_hint(&self, other: &dyn steel::rvals::CustomType) -> bool {
        as_underlying_type::<Self>(other).is_some_and(|o| o.0 == self.0)
    }

    fn try_as_dyn_hash(&self) -> Option<&dyn steel::rvals::DynHash> {
        Some(self)
    }
}

impl Custom for SteelBufferKey {
    fn fmt(&self) -> Option<Result<String, std::fmt::Error>> {
        Some(Ok(format!("#<buffer-key {}>", self.0.data().as_ffi())))
    }

    fn equality_hint(&self, other: &dyn steel::rvals::CustomType) -> bool {
        as_underlying_type::<Self>(other).is_some_and(|o| o.0 == self.0)
    }

    fn try_as_dyn_hash(&self) -> Option<&dyn steel::rvals::DynHash> {
        Some(self)
    }
}

// ── Predicate builtins ────────────────────────────────────────────────────────

/// `(pane? v)`: return `#t` if `v` is an opaque pane handle.
pub(crate) fn is_pane(val: SteelVal) -> bool {
    if let SteelVal::Custom(v) = &val {
        v.read().as_any_ref().downcast_ref::<SteelPane>().is_some()
    } else {
        false
    }
}

// ── Decode ────────────────────────────────────────────────────────────────────

pub(crate) fn downcast_pane(val: &SteelVal) -> Option<PaneHandle> {
    if let SteelVal::Custom(v) = val {
        v.read()
            .as_any_ref()
            .downcast_ref::<SteelPane>()
            .map(|p| p.0)
    } else {
        None
    }
}

#[cfg(test)]
mod tests;
