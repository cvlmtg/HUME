//! Total, bidirectional conversion between `serde_json::Value` and `SteelVal`.
//!
//! Mapping table:
//!
//! ```text
//! json -> steel:  null    -> Void        (NOT #f — false must round-trip distinctly)
//!                 bool    -> BoolV
//!                 number  -> IntV when i64-representable; BigNum when
//!                             u64-representable but not i64 (exact, no
//!                             precision loss); NumV (f64) otherwise
//!                 string  -> StringV
//!                 array   -> ListV
//!                 object  -> HashMapV with STRING keys (not symbols — JSON keys are
//!                             arbitrary data like "rust-analyzer.cargo", not identifiers)
//!
//! steel -> json:  inverse of the above; SymbolV is also accepted as a string
//!                 (Steel code may build hashmaps with symbol keys); anything
//!                 else unrepresentable (closures, custom types, ports, …)
//!                 is a hard error naming the offending value's kind.
//! ```
//!
//! This is deliberately generic JSON — it knows nothing about LSP shapes.

use num_traits::ToPrimitive;
use steel::HashMap as SteelHashMap;
use steel::gc::Gc;
use steel::rvals::SteelVal;

/// Converts a `serde_json::Value` into the equivalent `SteelVal`. Total —
/// every JSON value has a representation, so this never fails.
pub fn json_to_steel(v: &serde_json::Value) -> SteelVal {
    match v {
        serde_json::Value::Null => SteelVal::Void,
        serde_json::Value::Bool(b) => SteelVal::BoolV(*b),
        serde_json::Value::Number(n) => number_to_steel(n),
        serde_json::Value::String(s) => SteelVal::StringV(s.as_str().into()),
        serde_json::Value::Array(items) => {
            let items: Vec<SteelVal> = items.iter().map(json_to_steel).collect();
            SteelVal::ListV(items.into())
        }
        serde_json::Value::Object(map) => {
            let mut hm = SteelHashMap::new();
            for (key, value) in map {
                hm.insert(SteelVal::StringV(key.as_str().into()), json_to_steel(value));
            }
            SteelVal::HashMapV(Gc::new(hm).into())
        }
    }
}

fn number_to_steel(n: &serde_json::Number) -> SteelVal {
    if let Some(i) = n.as_i64() {
        SteelVal::IntV(i as isize)
    } else if let Some(u) = n.as_u64() {
        // In (i64::MAX, u64::MAX] — not i64-representable, but still an
        // exact integer (e.g. a large id/hash field). Falling back to f64
        // here would silently lose precision; BigNum represents it exactly
        // instead, so a value echoed back through steel_to_json still
        // matches what the server sent.
        SteelVal::BigNum(Gc::new(u.into()))
    } else {
        // Not representable as an integer at all — a genuine float.
        // serde_json::Number is always finite and, without the
        // `arbitrary_precision` feature (which this workspace does not
        // enable), always convertible to f64.
        SteelVal::NumV(
            n.as_f64()
                .expect("serde_json::Number is f64-representable without arbitrary_precision"),
        )
    }
}

/// Converts a `SteelVal` into the equivalent `serde_json::Value`. Fails on
/// values with no JSON representation (functions, ports, custom types, …) —
/// the error names the offending kind rather than silently producing `null`.
pub fn steel_to_json(v: &SteelVal) -> Result<serde_json::Value, String> {
    match v {
        SteelVal::Void => Ok(serde_json::Value::Null),
        SteelVal::BoolV(b) => Ok(serde_json::Value::Bool(*b)),
        SteelVal::IntV(i) => Ok(serde_json::Value::Number((*i as i64).into())),
        // Only ever produced (by json_to_steel) for a u64-range JSON integer,
        // so it always fits back into u64 exactly — but Steel code could in
        // principle construct a bigger one directly, hence the checked
        // conversion rather than an infallible one.
        SteelVal::BigNum(b) => b
            .to_u64()
            .map(|u| serde_json::Value::Number(u.into()))
            .ok_or_else(|| "integer too large to represent in JSON".to_string()),
        SteelVal::NumV(n) => serde_json::Number::from_f64(*n)
            .map(serde_json::Value::Number)
            .ok_or_else(|| format!("number is not finite: {n}")),
        SteelVal::StringV(s) => Ok(serde_json::Value::String(s.to_string())),
        SteelVal::SymbolV(s) => Ok(serde_json::Value::String(s.to_string())),
        SteelVal::ListV(items) => {
            let items = items
                .iter()
                .map(steel_to_json)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(serde_json::Value::Array(items))
        }
        SteelVal::HashMapV(hm) => {
            let mut map = serde_json::Map::new();
            for (key, value) in hm.iter() {
                let key = match key {
                    SteelVal::StringV(s) => s.to_string(),
                    SteelVal::SymbolV(s) => s.to_string(),
                    other => {
                        return Err(format!("hashmap key is not a string: {}", type_name(other)));
                    }
                };
                map.insert(key, steel_to_json(value)?);
            }
            Ok(serde_json::Value::Object(map))
        }
        other => Err(format!("cannot convert {} to JSON", type_name(other))),
    }
}

/// A short, readable label for error messages — not exhaustive over every
/// `SteelVal` variant, just the ones plausible enough to show up in a hashmap
/// or list a plugin author built by hand.
fn type_name(v: &SteelVal) -> &'static str {
    match v {
        SteelVal::Closure(_)
        | SteelVal::FuncV(_)
        | SteelVal::BoxedFunction(_)
        | SteelVal::MutFunc(_)
        | SteelVal::BuiltIn(_)
        | SteelVal::ContinuationFunction(_)
        | SteelVal::FutureFunc(_) => "function",
        SteelVal::Custom(_) | SteelVal::CustomStruct(_) => "custom type",
        SteelVal::PortV(_) => "port",
        SteelVal::HashSetV(_) => "hashset",
        SteelVal::VectorV(_) | SteelVal::MutableVector(_) => "vector",
        SteelVal::CharV(_) => "char",
        _ => "unsupported value",
    }
}

#[cfg(test)]
mod tests {
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
        // The float fallback this replaces would have rounded u64::MAX to
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
}
