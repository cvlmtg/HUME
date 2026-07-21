//! Opaque `BufferId` and `PaneId` Steel types for the scripting surface.
//!
//! Plugins receive and pass these values between builtins but cannot construct
//! or inspect them arithmetically — they are purely opaque handles.
//!
//! Display uses the slotmap `as_ffi` u64 so that `(log! "info" (current-buffer))`
//! prints something readable without revealing internal structure.

use hume_engine::pipeline::{BufferId, PaneId};
use slotmap::Key as _;
use steel::{
    gc::ShareableMut as _,
    rvals::{Custom, IntoSteelVal as _, SteelVal, as_underlying_type},
};

// ── Wrapper types ─────────────────────────────────────────────────────────────

/// Opaque Steel handle for a `BufferId`.
#[derive(Debug, Clone, PartialEq, Hash)]
pub struct SteelBufferId(pub(crate) BufferId);

impl SteelBufferId {
    /// Wrap a `BufferId` into a Steel-facing opaque handle.
    pub fn new(id: BufferId) -> Self {
        Self(id)
    }
}

/// Opaque Steel handle for a `PaneId`.
#[derive(Debug, Clone, PartialEq, Hash)]
pub(crate) struct SteelPaneId(pub(crate) PaneId);

impl SteelBufferId {
    /// Convert to a `SteelVal` without returning `Result`.
    ///
    /// `IntoSteelVal` for custom types is infallible; this avoids `.expect()` at
    /// every call site that wraps a `BufferId` for hook args or builtin returns.
    pub fn into_steel_val(self) -> SteelVal {
        self.into_steelval().expect("SteelBufferId into_steelval")
    }
}

impl SteelPaneId {
    /// Convert to a `SteelVal` without returning `Result`.
    ///
    /// Mirrors [`SteelBufferId::into_steel_val`] — `IntoSteelVal` for custom
    /// types is infallible.
    pub(crate) fn into_steel_val(self) -> SteelVal {
        self.into_steelval().expect("SteelPaneId into_steelval")
    }
}

impl Custom for SteelBufferId {
    fn fmt(&self) -> Option<Result<String, std::fmt::Error>> {
        Some(Ok(format!("#<buffer-id {}>", self.0.data().as_ffi())))
    }

    fn equality_hint(&self, other: &dyn steel::rvals::CustomType) -> bool {
        as_underlying_type::<Self>(other).is_some_and(|o| o.0 == self.0)
    }

    fn try_as_dyn_hash(&self) -> Option<&dyn steel::rvals::DynHash> {
        Some(self)
    }
}

impl Custom for SteelPaneId {
    fn fmt(&self) -> Option<Result<String, std::fmt::Error>> {
        Some(Ok(format!("#<pane-id {}>", self.0.data().as_ffi())))
    }

    fn equality_hint(&self, other: &dyn steel::rvals::CustomType) -> bool {
        as_underlying_type::<Self>(other).is_some_and(|o| o.0 == self.0)
    }

    fn try_as_dyn_hash(&self) -> Option<&dyn steel::rvals::DynHash> {
        Some(self)
    }
}

// ── Predicate builtins ────────────────────────────────────────────────────────

/// `(buffer-id? v)` — return `#t` if `v` is an opaque `BufferId`.
pub(crate) fn is_buffer_id(val: SteelVal) -> bool {
    if let SteelVal::Custom(v) = &val {
        v.read()
            .as_any_ref()
            .downcast_ref::<SteelBufferId>()
            .is_some()
    } else {
        false
    }
}

/// `(pane-id? v)` — return `#t` if `v` is an opaque `PaneId`.
pub(crate) fn is_pane_id(val: SteelVal) -> bool {
    if let SteelVal::Custom(v) = &val {
        v.read()
            .as_any_ref()
            .downcast_ref::<SteelPaneId>()
            .is_some()
    } else {
        false
    }
}

// ── Value-equality builtins ───────────────────────────────────────────────────
// `equal?` and hash-keying now compare by value (see the `Custom::equality_hint`
// / `try_as_dyn_hash` impls above) — a SteelBufferId can be used as a hash key
// and `equal?` returns `#t` for two wrappings of the same BufferId. These
// builtins are kept as an explicit, type-narrowed alternative for plugin code
// that only wants to compare ids and reject any other value outright.

pub(crate) fn downcast_buffer_id(val: &SteelVal) -> Option<BufferId> {
    if let SteelVal::Custom(v) = val {
        v.read()
            .as_any_ref()
            .downcast_ref::<SteelBufferId>()
            .map(|b| b.0)
    } else {
        None
    }
}

fn downcast_pane_id(val: &SteelVal) -> Option<PaneId> {
    if let SteelVal::Custom(v) = val {
        v.read()
            .as_any_ref()
            .downcast_ref::<SteelPaneId>()
            .map(|p| p.0)
    } else {
        None
    }
}

/// `(buffer-id=? a b)` — value-equality for opaque `BufferId` handles.
///
/// Returns `#t` if both `a` and `b` are buffer-ids wrapping the same
/// underlying `BufferId`.  Prefer this over `equal?`, which only returns
/// `#t` when both values share the same `Arc`.
pub(crate) fn buffer_id_equal(a: SteelVal, b: SteelVal) -> bool {
    matches!((downcast_buffer_id(&a), downcast_buffer_id(&b)), (Some(x), Some(y)) if x == y)
}

/// `(pane-id=? a b)` — value-equality for opaque `PaneId` handles.
pub(crate) fn pane_id_equal(a: SteelVal, b: SteelVal) -> bool {
    matches!((downcast_pane_id(&a), downcast_pane_id(&b)), (Some(x), Some(y)) if x == y)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
