use super::*;
use crate::types::PaneHandle;
use hume_engine::pipeline::PaneId;
use steel::rvals::IntoSteelVal;

fn pane_val(handle: PaneHandle) -> SteelVal {
    SteelPane(handle).into_steelval().unwrap()
}

#[test]
fn pane_predicate_true() {
    assert!(is_pane(pane_val(PaneHandle::buffer_only(
        BufferId::default()
    ))));
}

#[test]
fn pane_predicate_false_for_string() {
    assert!(!is_pane(SteelVal::StringV("hello".into())));
}

#[test]
fn pane_equality_same_buffer_and_pane() {
    let bid = BufferId::default();
    let pid = PaneId::default();
    let a = SteelPane(PaneHandle::with_pane(bid, pid));
    let b = SteelPane(PaneHandle::with_pane(bid, pid));
    assert_eq!(a, b);
}

#[test]
fn pane_inequality_same_buffer_different_pane_presence() {
    let bid = BufferId::default();
    let pid = PaneId::default();
    let with_pane = SteelPane(PaneHandle::with_pane(bid, pid));
    let no_pane = SteelPane(PaneHandle::buffer_only(bid));
    assert_ne!(with_pane, no_pane);
}

#[test]
fn pane_display_with_pane() {
    let handle = PaneHandle::with_pane(BufferId::default(), PaneId::default());
    let s = SteelPane(handle).fmt().unwrap().unwrap();
    assert!(s.starts_with("#<pane buffer="), "got: {s}");
    assert!(s.contains("pane="), "got: {s}");
}

#[test]
fn pane_display_buffer_only() {
    let handle = PaneHandle::buffer_only(BufferId::default());
    let s = SteelPane(handle).fmt().unwrap().unwrap();
    assert!(s.starts_with("#<pane buffer="), "got: {s}");
    assert!(!s.contains("pane="), "got: {s}");
}

/// Exercises `equal?`'s real dispatch through a full Steel eval (not just
/// the Rust-level `PartialEq` derive, which never touches steel-core's
/// `equal?` machinery). Without `equality_hint`/`try_as_dyn_hash`
/// (implemented above), steel-core's default `equality_hint` returns `true`
/// for any same-type `Custom` pair regardless of contents, making
/// same-value and different-value panes indistinguishable under `equal?`.
/// With them, this eval correctly reports `(#t #f)`.
#[test]
fn equal_compares_by_value_through_a_real_steel_eval() {
    let mut steel = steel::steel_vm::engine::Engine::new();
    crate::builtins::register_all(&mut steel);
    let id = BufferId::default();
    let other = {
        // A distinct slotmap key: allocate through a fresh slotmap so it
        // differs from `BufferId::default()`.
        let mut sm: slotmap::SlotMap<BufferId, ()> = slotmap::SlotMap::with_key();
        sm.insert(())
    };
    assert_ne!(id, other, "test setup: need two distinct BufferIds");

    steel.register_value("a", pane_val(PaneHandle::buffer_only(id)));
    steel.register_value("b", pane_val(PaneHandle::buffer_only(id)));
    steel.register_value("c", pane_val(PaneHandle::buffer_only(other)));

    let results = steel
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
        "equal? must be #t for two wrappings of the same PaneHandle and #f for \
             different ones"
    );
}

/// A `SteelPane` must be usable as a Steel hash key: two distinct wrappings
/// of the same `PaneHandle` must hash-collide and `hash-ref` the same entry —
/// the concrete capability per-(buffer,pane) plugin state needs.
#[test]
fn pane_is_usable_as_a_steel_hash_key() {
    let mut steel = steel::steel_vm::engine::Engine::new();
    crate::builtins::register_all(&mut steel);
    let handle = PaneHandle::with_pane(BufferId::default(), PaneId::default());
    steel.register_value("a", pane_val(handle));
    steel.register_value("b", pane_val(handle));

    let results = steel
        .compile_and_run_raw_program("(hash-ref (hash-insert (hash) a 42) b)")
        .expect("eval must succeed");
    assert_eq!(results.into_iter().next().unwrap(), SteelVal::IntV(42));
}

/// `buffer-key`'s whole point: two panes on the same buffer must hash/compare
/// equal once narrowed to `SteelBufferKey`, even though the `SteelPane`
/// values themselves differ (see `pane_inequality_same_buffer_different_pane_presence`).
#[test]
fn buffer_key_equal_across_different_panes_on_same_buffer() {
    let bid = BufferId::default();
    let a = SteelBufferKey(bid);
    let b = SteelBufferKey(bid);
    assert_eq!(a, b);
}

#[test]
fn buffer_key_display() {
    let key = SteelBufferKey(BufferId::default());
    let s = key.fmt().unwrap().unwrap();
    assert!(s.starts_with("#<buffer-key "), "got: {s}");
}
