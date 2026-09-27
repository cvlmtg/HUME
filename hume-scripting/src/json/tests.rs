use super::*;
use serde_json::json;

fn round_trip(v: serde_json::Value) {
    let steel = json_to_steel(&v);
    let back = steel_to_json(&steel).expect("round trip must succeed");
    assert_eq!(back, v, "round trip mismatch for {v:?}");
}

#[test]
fn round_trips_scalars_distinctly() {
    round_trip(json!(null));
    round_trip(json!(false));
    round_trip(json!(true));
    round_trip(json!(0));
    round_trip(json!(""));
    round_trip(json!("hello"));

    // null, false, 0 and "" must all be distinct after conversion.
    let n = json_to_steel(&json!(null));
    let f = json_to_steel(&json!(false));
    let zero = json_to_steel(&json!(0));
    let empty = json_to_steel(&json!(""));
    assert!(matches!(n, SteelVal::Void));
    assert!(matches!(f, SteelVal::BoolV(false)));
    assert!(matches!(zero, SteelVal::IntV(0)));
    assert!(matches!(empty, SteelVal::StringV(ref s) if s.as_str().is_empty()));
}

#[test]
fn round_trips_integers_and_floats() {
    round_trip(json!(i64::MAX));
    round_trip(json!(i64::MIN));
    round_trip(json!(3.5));
    round_trip(json!(-0.125));

    assert!(matches!(json_to_steel(&json!(42)), SteelVal::IntV(42)));
    assert!(matches!(json_to_steel(&json!(1.5)), SteelVal::NumV(n) if n == 1.5));
}

/// Regression: a JSON integer in `(i64::MAX, u64::MAX]` (e.g. a large
/// id/hash field) must round-trip exactly, not silently lose precision
/// through an f64 fallback.
#[test]
fn round_trips_u64_range_integers_exactly_via_bignum() {
    let huge = u64::MAX; // 18446744073709551615 — not i64- or f64-exact-representable
    round_trip(json!(huge));

    let steel = json_to_steel(&json!(huge));
    assert!(
        matches!(steel, SteelVal::BigNum(_)),
        "expected BigNum for a u64-range integer, got {steel:?}"
    );
    // An f64 fallback would round u64::MAX to 18446744073709551616.0, since
    // f64 can't represent every u64 exactly. The round trip must land on the
    // exact original value.
    assert_eq!(
        steel_to_json(&steel).unwrap(),
        json!(huge),
        "must not lose precision the way an f64 fallback would"
    );
}

#[test]
fn round_trips_unicode_strings() {
    round_trip(json!("héllo wörld 🎉"));
}

#[test]
fn round_trips_nested_arrays_and_objects() {
    round_trip(json!({
        "a": [1, 2, 3],
        "b": { "nested": true, "rust-analyzer.cargo": null },
        "c": [],
        "d": {}
    }));
}

#[test]
fn object_keys_are_strings_not_symbols() {
    let steel = json_to_steel(&json!({"foo": 1}));
    match steel {
        SteelVal::HashMapV(hm) => {
            let key = hm.keys().next().expect("one key");
            assert!(
                matches!(key, SteelVal::StringV(_)),
                "expected StringV key, got {key:?}"
            );
        }
        other => panic!("expected HashMapV, got {other:?}"),
    }
}

#[test]
fn reverse_accepts_symbol_keys() {
    let mut hm = SteelHashMap::new();
    hm.insert(SteelVal::SymbolV("foo".into()), SteelVal::IntV(1));
    let steel = SteelVal::HashMapV(Gc::new(hm).into());
    let json = steel_to_json(&steel).expect("symbol keys are accepted");
    assert_eq!(json, json!({"foo": 1}));
}

#[test]
fn reverse_rejects_closures_naming_the_type() {
    let err = steel_to_json(&SteelVal::FuncV(|_| unreachable!())).unwrap_err();
    assert!(
        err.contains("function"),
        "error should name the type: {err}"
    );
}

#[test]
fn flip_check_object_keys_would_fail_if_symbols_leaked() {
    // If the impl regressed to SymbolV keys, this assertion catches it.
    let steel = json_to_steel(&json!({"foo": 1}));
    match steel {
        SteelVal::HashMapV(hm) => {
            let key = hm.keys().next().unwrap();
            assert_ne!(
                std::mem::discriminant(key),
                std::mem::discriminant(&SteelVal::SymbolV("foo".into()))
            );
        }
        _ => unreachable!(),
    }
}

// ── JsonHandle ────────────────────────────────────────────────────────────────

#[test]
fn json_handle_round_trips_through_a_steel_val() {
    let v = json!({"items": [1, 2, 3], "isIncomplete": true});
    let handle = JsonHandle::new(v.clone());
    let steel = handle.into_steel_val();
    let back = downcast_json_handle(&steel).expect("must downcast back to a JsonHandle");
    assert_eq!(back.value(), &v);
}

#[test]
fn downcast_json_handle_rejects_an_unrelated_value() {
    assert!(downcast_json_handle(&SteelVal::IntV(1)).is_none());
    let other = json_to_steel(&json!({"foo": 1}));
    assert!(
        downcast_json_handle(&other).is_none(),
        "an ordinary converted hashmap is not a JsonHandle"
    );
}

/// Real `equal?` dispatch through the Steel VM, not just Rust-level
/// `PartialEq` on `serde_json::Value` — without `equality_hint`/
/// `try_as_dyn_hash`, steel-core's default `equality_hint` returns `true`
/// for any two `Custom` values of the same type regardless of contents,
/// which is exactly the bug this test would catch.
#[test]
fn equal_compares_by_resolved_value_through_a_real_steel_eval() {
    let mut steel = steel::steel_vm::engine::Engine::new();
    crate::builtins::register_all(&mut steel);
    steel.register_value("a", JsonHandle::new(json!({"x": 1})).into_steel_val());
    steel.register_value("b", JsonHandle::new(json!({"x": 1})).into_steel_val());
    steel.register_value("c", JsonHandle::new(json!({"x": 2})).into_steel_val());

    let results = steel
        .compile_and_run_raw_program("(list (equal? a b) (equal? a c))")
        .expect("eval must succeed");
    let SteelVal::ListV(items) = results.into_iter().next().unwrap() else {
        panic!("expected a list result");
    };
    let items: Vec<_> = items.into_iter().collect();
    assert_eq!(
        items,
        vec![SteelVal::BoolV(true), SteelVal::BoolV(false)],
        "equal? must compare by resolved JSON value, not handle identity"
    );
}

/// Object key order must not affect equality or hashing — `serde_json`'s
/// `Map` (backed by `indexmap` under this workspace's `preserve_order`
/// feature) already compares order-insensitively; this pins that a
/// `JsonHandle` built over a differently-ordered object still `equal?`s.
#[test]
fn equal_ignores_object_key_order() {
    let mut steel = steel::steel_vm::engine::Engine::new();
    crate::builtins::register_all(&mut steel);
    steel.register_value(
        "a",
        JsonHandle::new(json!({"x": 1, "y": 2})).into_steel_val(),
    );
    steel.register_value(
        "b",
        JsonHandle::new(json!({"y": 2, "x": 1})).into_steel_val(),
    );

    let results = steel
        .compile_and_run_raw_program("(equal? a b)")
        .expect("eval must succeed");
    assert_eq!(results.into_iter().next().unwrap(), SteelVal::BoolV(true));
}

/// A `JsonHandle` must be usable as a Steel hash key — `equality_hint` alone
/// is not enough; `try_as_dyn_hash` must agree with it.
#[test]
fn json_handle_is_usable_as_a_steel_hash_key() {
    let mut steel = steel::steel_vm::engine::Engine::new();
    crate::builtins::register_all(&mut steel);
    steel.register_value("a", JsonHandle::new(json!({"x": 1})).into_steel_val());
    steel.register_value("b", JsonHandle::new(json!({"x": 1})).into_steel_val());

    let results = steel
        .compile_and_run_raw_program("(hash-ref (hash-insert (hash) a 42) b)")
        .expect("eval must succeed");
    assert_eq!(results.into_iter().next().unwrap(), SteelVal::IntV(42));
}

/// `-0.0` and `0.0` compare equal under `serde_json::Number`'s `PartialEq`
/// but print as different strings — hashing via `Number`'s own derived
/// `Hash` (rather than its `Display`) keeps `try_as_dyn_hash` consistent
/// with `equality_hint` for this pair the way it already is for every
/// other Number.
#[test]
fn negative_zero_and_zero_handles_share_a_hash_key() {
    let mut steel = steel::steel_vm::engine::Engine::new();
    crate::builtins::register_all(&mut steel);
    steel.register_value("a", JsonHandle::new(json!({"x": -0.0})).into_steel_val());
    steel.register_value("b", JsonHandle::new(json!({"x": 0.0})).into_steel_val());

    let results = steel
        .compile_and_run_raw_program("(hash-ref (hash-insert (hash) a 42) b)")
        .expect("eval must succeed");
    assert_eq!(results.into_iter().next().unwrap(), SteelVal::IntV(42));
}

/// A sub-handle produced by navigating a container must share its parent's
/// root `Arc` rather than cloning the subtree — the whole point of rooting
/// a handle instead of wrapping each navigation result independently.
#[test]
fn navigating_a_container_shares_the_root_arc() {
    let root = std::sync::Arc::new(json!({"a": {"b": 1}}));
    let handle = downcast_json_handle(&to_steel_handle(
        std::sync::Arc::clone(&root),
        WireOrigin::Local,
    ))
    .expect("an object roots as a JsonHandle");
    let sub = handle
        .resolve(&[Seg::Key("a".into())], "test")
        .expect("path resolves");
    let SteelVal::Custom(_) = &sub else {
        panic!("expected a sub-handle for a container field");
    };
    let sub_handle = downcast_json_handle(&sub).unwrap();
    assert!(
        std::sync::Arc::ptr_eq(&sub_handle.root, &root),
        "sub-handle must share the parent's root Arc, not clone the subtree"
    );
}

/// `resolve` on a scalar field returns a native Steel value, not a handle.
#[test]
fn resolving_a_scalar_field_returns_a_native_value() {
    let handle = JsonHandle::new(json!({"a": 5}));
    let result = handle
        .resolve(&[Seg::Key("a".into())], "test")
        .expect("path resolves");
    assert_eq!(result, SteelVal::IntV(5));
}

/// `resolve` names the failing path in its error.
#[test]
fn resolve_error_names_the_path() {
    let handle = JsonHandle::new(json!({"a": {}}));
    let err = handle
        .resolve(&[Seg::Key("a".into()), Seg::Key("missing".into())], "test")
        .unwrap_err();
    assert!(err.contains("$.a"), "got: {err}");
    assert!(err.contains("missing"), "got: {err}");
}

#[test]
fn contains_true_for_null_present_field() {
    let handle = JsonHandle::new(json!({"a": null}));
    assert!(handle.contains(&[Seg::Key("a".into())]));
}

#[test]
fn contains_false_for_absent_field() {
    let handle = JsonHandle::new(json!({"a": 1}));
    assert!(!handle.contains(&[Seg::Key("z".into())]));
}

// ── WireOrigin propagation ──────────────────────────────────────────────────

/// `JsonHandle::new` — every hand-built handle (a `json-parse` result, a
/// plugin's own hashmap) is untagged: `position_encoding` refuses.
#[test]
fn new_is_untagged() {
    let handle = JsonHandle::new(json!({"a": 1}));
    assert!(handle.position_encoding("test").is_err());
}

/// A server-tagged handle reports the encoding it was tagged with.
#[test]
fn server_for_test_reports_its_tagged_encoding() {
    let handle = JsonHandle::server_for_test(json!({"a": 1}), PositionEncoding::Utf8);
    assert_eq!(handle.position_encoding("test"), Ok(PositionEncoding::Utf8));
}

/// A child produced by `resolve` (`json-ref`'s implementation) inherits its
/// parent's tag.
#[test]
fn resolve_inherits_the_parent_tag() {
    let handle = JsonHandle::server_for_test(json!({"a": {"b": 1}}), PositionEncoding::Utf16);
    let resolved = handle
        .resolve(&[Seg::Key("a".into())], "test")
        .expect("path resolves");
    let child = downcast_json_handle(&resolved).expect("a container field resolves to a handle");
    assert_eq!(child.position_encoding("test"), Ok(PositionEncoding::Utf16));
}

/// A child produced by `lookup` (`json-ref-or`'s implementation) inherits
/// its parent's tag.
#[test]
fn lookup_inherits_the_parent_tag() {
    let handle = JsonHandle::server_for_test(json!({"a": {"b": 1}}), PositionEncoding::Utf8);
    let child = downcast_json_handle(
        &handle
            .lookup(&[Seg::Key("a".into())])
            .expect("path resolves"),
    )
    .unwrap();
    assert_eq!(child.position_encoding("test"), Ok(PositionEncoding::Utf8));
}

/// Every element `list_items` (`json-list`'s implementation) pulls out of an
/// array inherits the parent's tag.
#[test]
fn list_items_inherit_the_parent_tag() {
    let handle = JsonHandle::server_for_test(json!([{"a": 1}, {"b": 2}]), PositionEncoding::Utf16);
    for item in handle.list_items("test").expect("array") {
        let child = downcast_json_handle(&item).unwrap();
        assert_eq!(child.position_encoding("test"), Ok(PositionEncoding::Utf16));
    }
}

/// `indexed_child` (the Rust-side slice a `textDocument/completion` response
/// hands off) inherits the parent's tag.
#[test]
fn indexed_child_inherits_the_parent_tag() {
    let handle = JsonHandle::server_for_test(json!({"items": [{"a": 1}]}), PositionEncoding::Utf8);
    let child = handle
        .indexed_child(Some("items"), 0)
        .expect("path resolves");
    assert_eq!(child.position_encoding("test"), Ok(PositionEncoding::Utf8));
}
