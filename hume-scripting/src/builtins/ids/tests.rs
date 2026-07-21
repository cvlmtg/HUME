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
/// `equal?` machinery). Without `equality_hint`/`try_as_dyn_hash`
/// (implemented above), steel-core's default `equality_hint` returns `true`
/// for any same-type `Custom` pair regardless of contents, making
/// same-value and different-value ids indistinguishable under `equal?`.
/// With them, this eval correctly reports `(#t #f)`.
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
