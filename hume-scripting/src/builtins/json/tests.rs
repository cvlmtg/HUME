use super::*;
use crate::json::JsonHandle;
use serde_json::json;
use steel::rvals::IntoSteelVal;

/// Builds the `&[SteelVal]` a `FuncV` accessor sees: `handle` followed by
/// its path segments.
fn call_args(handle: SteelVal, segs: Vec<SteelVal>) -> Vec<SteelVal> {
    std::iter::once(handle).chain(segs).collect()
}

#[test]
fn parses_a_nested_object_into_a_json_handle() {
    let result = json_parse(r#"{"a": [1, 2], "b": {"c": true}}"#.into_steelval().unwrap())
        .expect("well-formed JSON must parse");
    let handle = downcast_json_handle(&result).expect("top-level object must be a JsonHandle");
    assert_eq!(handle.value(), &json!({"a": [1, 2], "b": {"c": true}}));
}

#[test]
fn parses_a_top_level_scalar_natively() {
    let result = json_parse("42".into_steelval().unwrap()).expect("well-formed JSON must parse");
    assert_eq!(result, SteelVal::IntV(42));
}

#[test]
fn parses_top_level_null_as_void() {
    let result = json_parse("null".into_steelval().unwrap()).expect("well-formed JSON must parse");
    assert!(matches!(result, SteelVal::Void));
}

#[test]
fn raises_on_malformed_json() {
    let err = json_parse("not json".into_steelval().unwrap()).unwrap_err();
    assert!(err.to_string().contains("json-parse"), "got: {err}");
}

#[test]
fn raises_on_non_string_argument() {
    let err = json_parse(SteelVal::IntV(1)).unwrap_err();
    assert!(err.to_string().contains("json-parse"), "got: {err}");
}

// ── JsonHandle accessors ──────────────────────────────────────────────────────

fn handle(v: serde_json::Value) -> SteelVal {
    JsonHandle::new(v).into_steel_val()
}

#[test]
fn json_ref_reads_a_scalar_field() {
    let h = handle(json!({"a": {"b": 5}}));
    let args = call_args(
        h,
        vec![SteelVal::StringV("a".into()), SteelVal::StringV("b".into())],
    );
    let result = json_ref(&args).expect("path resolves");
    assert_eq!(result, SteelVal::IntV(5));
}

#[test]
fn json_ref_returns_a_sub_handle_for_a_container() {
    let h = handle(json!({"a": {"b": 5}}));
    let args = call_args(h, vec![SteelVal::StringV("a".into())]);
    let result = json_ref(&args).expect("path resolves");
    let sub = crate::json::downcast_json_handle(&result).expect("container -> sub-handle");
    assert_eq!(sub.value(), &json!({"b": 5}));
}

#[test]
fn json_ref_indexes_arrays_by_integer() {
    let h = handle(json!({"items": [10, 20, 30]}));
    let args = call_args(
        h,
        vec![SteelVal::StringV("items".into()), SteelVal::IntV(1)],
    );
    let result = json_ref(&args).expect("path resolves");
    assert_eq!(result, SteelVal::IntV(20));
}

#[test]
fn json_ref_null_field_is_void() {
    let h = handle(json!({"a": null}));
    let args = call_args(h, vec![SteelVal::StringV("a".into())]);
    let result = json_ref(&args).expect("path resolves");
    assert!(matches!(result, SteelVal::Void));
}

#[test]
fn json_ref_raises_on_missing_key_naming_the_path() {
    let h = handle(json!({"a": {}}));
    let args = call_args(
        h,
        vec![
            SteelVal::StringV("a".into()),
            SteelVal::StringV("missing".into()),
        ],
    );
    let err = json_ref(&args).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("missing"), "got: {msg}");
    assert!(
        msg.contains("$.a"),
        "error should name the path so far, got: {msg}"
    );
}

#[test]
fn json_ref_raises_on_out_of_range_index() {
    let h = handle(json!({"items": [1]}));
    let args = call_args(
        h,
        vec![SteelVal::StringV("items".into()), SteelVal::IntV(5)],
    );
    let err = json_ref(&args).unwrap_err();
    assert!(err.to_string().contains("out of range"));
}

#[test]
fn json_ref_raises_on_wrong_container_kind() {
    let h = handle(json!({"a": [1, 2]}));
    // "a" is an array — looking up a string key on it is a kind mismatch.
    let args = call_args(
        h,
        vec![SteelVal::StringV("a".into()), SteelVal::StringV("b".into())],
    );
    let err = json_ref(&args).unwrap_err();
    assert!(err.to_string().contains("array"));
}

#[test]
fn json_ref_rejects_a_non_handle() {
    let args = call_args(SteelVal::IntV(1), vec![SteelVal::StringV("a".into())]);
    let err = json_ref(&args).unwrap_err();
    assert!(err.to_string().contains("json-ref"));
}

#[test]
fn json_ref_raises_on_no_path_segments() {
    let err = json_ref(&[handle(json!({"a": 1}))]).unwrap_err();
    assert!(err.to_string().contains("json-ref"));
}

#[test]
fn json_contains_true_for_a_present_path_including_null() {
    let h = handle(json!({"a": null, "b": 1}));
    let a_args = call_args(h.clone(), vec![SteelVal::StringV("a".into())]);
    assert_eq!(json_contains(&a_args).unwrap(), SteelVal::BoolV(true));
    let b_args = call_args(h, vec![SteelVal::StringV("b".into())]);
    assert_eq!(json_contains(&b_args).unwrap(), SteelVal::BoolV(true));
}

#[test]
fn json_contains_false_for_a_missing_path() {
    let h = handle(json!({"a": 1}));
    let args = call_args(h, vec![SteelVal::StringV("z".into())]);
    let result = json_contains(&args).unwrap();
    assert_eq!(result, SteelVal::BoolV(false));
}

#[test]
fn json_ref_or_returns_the_value_when_present() {
    let h = handle(json!({"a": 5}));
    let args = call_args(
        h,
        vec![SteelVal::BoolV(false), SteelVal::StringV("a".into())],
    );
    assert_eq!(json_ref_or(&args).unwrap(), SteelVal::IntV(5));
}

#[test]
fn json_ref_or_returns_void_for_a_present_null() {
    let h = handle(json!({"a": null}));
    let args = call_args(h, vec![SteelVal::IntV(9), SteelVal::StringV("a".into())]);
    assert!(matches!(json_ref_or(&args).unwrap(), SteelVal::Void));
}

#[test]
fn json_ref_or_returns_the_default_on_a_missing_key() {
    let h = handle(json!({"a": 1}));
    let args = call_args(
        h,
        vec![SteelVal::IntV(9), SteelVal::StringV("missing".into())],
    );
    assert_eq!(json_ref_or(&args).unwrap(), SteelVal::IntV(9));
}

#[test]
fn json_ref_or_returns_the_default_through_wrong_container_kind() {
    let h = handle(json!({"a": [1, 2]}));
    let args = call_args(
        h,
        vec![
            SteelVal::IntV(9),
            SteelVal::StringV("a".into()),
            SteelVal::StringV("b".into()),
        ],
    );
    assert_eq!(json_ref_or(&args).unwrap(), SteelVal::IntV(9));
}

#[test]
fn json_ref_or_rejects_a_non_handle() {
    let args = call_args(
        SteelVal::IntV(1),
        vec![SteelVal::BoolV(false), SteelVal::StringV("a".into())],
    );
    let err = json_ref_or(&args).unwrap_err();
    assert!(err.to_string().contains("json-ref-or"));
}

#[test]
fn json_list_gives_sub_handles_and_scalars() {
    let arr = handle(json!([1, {"x": true}, "s"]));
    let list = json_list(arr).expect("array handle");
    let SteelVal::ListV(items) = list else {
        panic!("expected a list");
    };
    let items: Vec<_> = items.into_iter().collect();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0], SteelVal::IntV(1));
    let sub = crate::json::downcast_json_handle(&items[1]).expect("object element -> sub-handle");
    assert_eq!(sub.value(), &json!({"x": true}));
    assert_eq!(items[2], SteelVal::StringV("s".into()));
}

#[test]
fn json_list_raises_on_a_non_array() {
    let h = handle(json!({"a": 1}));
    let err = json_list(h).unwrap_err();
    assert!(err.to_string().contains("array"));
}

#[test]
fn json_array_and_object_predicates() {
    assert!(is_json_array(handle(json!([1, 2]))));
    assert!(!is_json_object(handle(json!([1, 2]))));
    assert!(is_json_object(handle(json!({"a": 1}))));
    assert!(!is_json_array(handle(json!({"a": 1}))));
    assert!(!is_json_array(SteelVal::IntV(1)));
    assert!(!is_json_object(SteelVal::IntV(1)));
}
