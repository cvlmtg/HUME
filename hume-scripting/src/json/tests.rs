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
    // Fail oracle: an f64 fallback would round u64::MAX to
    // 18446744073709551616.0 (f64 can't represent every u64 exactly) —
    // confirm the round trip lands on the exact original value, not that.
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
