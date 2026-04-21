//! Opaque `BufferId` and `PaneId` Steel types for the scripting surface.
//!
//! Plugins receive and pass these values between builtins but cannot construct
//! or inspect them arithmetically — they are purely opaque handles.
//!
//! Display uses the slotmap `as_ffi` u64 so that `(log! "info" (current-buffer))`
//! prints something readable without revealing internal structure.

use engine::pipeline::{BufferId, PaneId};
use slotmap::Key as _;
use steel::{
    gc::ShareableMut as _,
    rvals::{Custom, IntoSteelVal as _, SteelVal},
};

// ── Wrapper types ─────────────────────────────────────────────────────────────

/// Opaque Steel handle for a `BufferId`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SteelBufferId(pub(crate) BufferId);

/// Opaque Steel handle for a `PaneId`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SteelPaneId(pub(crate) PaneId);

impl SteelBufferId {
    /// Convert to a `SteelVal` without returning `Result`.
    ///
    /// `IntoSteelVal` for custom types is infallible; this avoids `.expect()` at
    /// every call site that wraps a `BufferId` for hook args or builtin returns.
    pub(crate) fn into_steel_val(self) -> SteelVal {
        self.into_steelval().expect("SteelBufferId into_steelval")
    }
}

impl Custom for SteelBufferId {
    fn fmt(&self) -> Option<Result<String, std::fmt::Error>> {
        Some(Ok(format!("#<buffer-id {}>", self.0.data().as_ffi())))
    }
}

impl Custom for SteelPaneId {
    fn fmt(&self) -> Option<Result<String, std::fmt::Error>> {
        Some(Ok(format!("#<pane-id {}>", self.0.data().as_ffi())))
    }
}

// ── Predicate builtins ────────────────────────────────────────────────────────

/// `(buffer-id? v)` — return `#t` if `v` is an opaque `BufferId`.
pub(crate) fn is_buffer_id(val: SteelVal) -> bool {
    if let SteelVal::Custom(v) = &val {
        v.read().as_any_ref().downcast_ref::<SteelBufferId>().is_some()
    } else {
        false
    }
}

/// `(pane-id? v)` — return `#t` if `v` is an opaque `PaneId`.
pub(crate) fn is_pane_id(val: SteelVal) -> bool {
    if let SteelVal::Custom(v) = &val {
        v.read().as_any_ref().downcast_ref::<SteelPaneId>().is_some()
    } else {
        false
    }
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
}
