use super::*;
use steel::rvals::IntoSteelVal;

#[test]
fn parses_a_nested_object() {
    let result = json_parse(r#"{"a": [1, 2], "b": {"c": true}}"#.into_steelval().unwrap())
        .expect("well-formed JSON must parse");
    let SteelVal::HashMapV(hm) = result else {
        panic!("expected a hashmap");
    };
    assert_eq!(
        hm.get(&SteelVal::StringV("a".into())),
        Some(&SteelVal::ListV(
            vec![SteelVal::IntV(1), SteelVal::IntV(2)].into()
        ))
    );
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
