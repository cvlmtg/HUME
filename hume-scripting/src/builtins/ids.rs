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
mod tests {
    use super::*;
    use steel::rvals::IntoSteelVal;

    fn buffer_id_val() -> SteelVal {
        SteelBufferId(BufferId::default()).into_steelval().unwrap()
    }

    fn pane_id_val() -> SteelVal {
        SteelPaneId(PaneId::default()).into_steelval().unwrap()
    }

    #[test]
    fn buffer_id_predicate_true() {
        assert!(is_buffer_id(buffer_id_val()));
    }

    #[test]
    fn buffer_id_predicate_false_for_pane() {
        assert!(!is_buffer_id(pane_id_val()));
    }

    #[test]
    fn buffer_id_predicate_false_for_string() {
        assert!(!is_buffer_id(SteelVal::StringV("hello".into())));
    }

    #[test]
    fn pane_id_predicate_true() {
        assert!(is_pane_id(pane_id_val()));
    }

    #[test]
    fn pane_id_predicate_false_for_buffer() {
        assert!(!is_pane_id(buffer_id_val()));
    }

    #[test]
    fn buffer_id_equality() {
        let a = SteelBufferId(BufferId::default());
        let b = SteelBufferId(BufferId::default());
        assert_eq!(a, b);
    }

    #[test]
    fn pane_id_equality() {
        let a = SteelPaneId(PaneId::default());
        let b = SteelPaneId(PaneId::default());
        assert_eq!(a, b);
    }

    #[test]
    fn buffer_id_display() {
        let id = SteelBufferId(BufferId::default());
        let s = id.fmt().unwrap().unwrap();
        assert!(s.starts_with("#<buffer-id "), "got: {s}");
    }

    #[test]
    fn pane_id_display() {
        let id = SteelPaneId(PaneId::default());
        let s = id.fmt().unwrap().unwrap();
        assert!(s.starts_with("#<pane-id "), "got: {s}");
    }

    /// Exercises `equal?`'s real dispatch through a full Steel eval (not just
    /// the Rust-level `PartialEq` derive, which never touches steel-core's
    /// `equal?` machinery). Before `equality_hint`/`try_as_dyn_hash` were
    /// implemented above, steel-core's default `equality_hint` returned `true`
    /// for any same-type `Custom` pair regardless of contents, so this exact
    /// eval used to report `(#t #t)` — same-value and different-value ids were
    /// indistinguishable under `equal?`. It now reports `(#t #f)`.
    #[test]
    fn equal_compares_by_value_through_a_real_steel_eval() {
        let mut engine = steel::steel_vm::engine::Engine::new();
        crate::builtins::register_all(&mut engine);
        let id = BufferId::default();
        let other = {
            // A distinct slotmap key: allocate through a fresh slotmap so it
            // differs from `BufferId::default()`.
            let mut sm: slotmap::SlotMap<BufferId, ()> = slotmap::SlotMap::with_key();
            sm.insert(())
        };
        assert_ne!(id, other, "test setup: need two distinct BufferIds");

        engine.register_value("a", SteelBufferId(id).into_steelval().unwrap());
        engine.register_value("b", SteelBufferId(id).into_steelval().unwrap());
        engine.register_value("c", SteelBufferId(other).into_steelval().unwrap());

        let results = engine
            .compile_and_run_raw_program("(list (equal? a b) (equal? a c))")
            .expect("eval must succeed");
        let list = results.into_iter().next().unwrap();
        let SteelVal::ListV(items) = list else {
            panic!("expected a list result");
        };
        let items: Vec<_> = items.into_iter().collect();
        assert_eq!(
            items,
            vec![SteelVal::BoolV(true), SteelVal::BoolV(false)],
            "equal? must be #t for two wrappings of the same BufferId and #f for \
             different BufferIds"
        );
    }

    /// A `SteelBufferId` must be usable as a Steel hash key: two distinct
    /// wrappings of the same `BufferId` must hash-collide and `hash-ref` the
    /// same entry — the concrete capability R3/D's per-buffer state needs.
    #[test]
    fn buffer_id_is_usable_as_a_steel_hash_key() {
        let mut engine = steel::steel_vm::engine::Engine::new();
        crate::builtins::register_all(&mut engine);
        let id = BufferId::default();
        engine.register_value("a", SteelBufferId(id).into_steelval().unwrap());
        engine.register_value("b", SteelBufferId(id).into_steelval().unwrap());

        let results = engine
            .compile_and_run_raw_program("(hash-ref (hash-insert (hash) a 42) b)")
            .expect("eval must succeed");
        assert_eq!(results.into_iter().next().unwrap(), SteelVal::IntV(42));
    }

    #[test]
    fn buffer_id_equal_same_value() {
        // Two different SteelVal wrappings of the same BufferId must be equal.
        let id = BufferId::default();
        let a = SteelBufferId(id).into_steelval().unwrap();
        let b = SteelBufferId(id).into_steelval().unwrap();
        assert!(buffer_id_equal(a, b), "same BufferId must be equal");
    }

    #[test]
    fn buffer_id_equal_rejects_wrong_type() {
        let a = SteelBufferId(BufferId::default()).into_steelval().unwrap();
        let b = SteelVal::BoolV(true);
        assert!(!buffer_id_equal(a, b));
    }

    #[test]
    fn pane_id_equal_same_value() {
        let id = PaneId::default();
        let a = SteelPaneId(id).into_steelval().unwrap();
        let b = SteelPaneId(id).into_steelval().unwrap();
        assert!(pane_id_equal(a, b), "same PaneId must be equal");
    }

    #[test]
    fn pane_id_equal_rejects_wrong_type() {
        let a = SteelPaneId(PaneId::default()).into_steelval().unwrap();
        let b = SteelVal::BoolV(false);
        assert!(!pane_id_equal(a, b));
    }
}
