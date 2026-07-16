//! `(json-parse str)` — general-purpose JSON string decoding for Steel.
//!
//! Not LSP-specific: any plugin data pipeline that embeds a JSON blob as a
//! Scheme string literal (rather than reconstructing the same structure as
//! nested Scheme data) needs this to get it back out. `core:lsp`'s seeded
//! server catalog (`registration.scm`) is the first caller — settings are
//! generated as a single canonical JSON string rather than a nested
//! tagged-alist/vector-array Scheme literal.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::json::json_to_steel;

use super::args::string_arg;
use super::errors::generic_err;

/// `(json-parse str)` -> the decoded value, via the same `json_to_steel`
/// mapping `lsp-request` responses already use. No context gate — pure data
/// parsing, callable from init.scm, plugin load, or a command/hook body
/// alike. Raises (does not silently return `#f`) on malformed JSON: a
/// corrupt seeded data file is a build-time bug, not a runtime condition to
/// tolerate.
pub(crate) fn json_parse(s: SteelVal) -> Result<SteelVal, SteelErr> {
    let s = string_arg(s, "json-parse")?;
    let value: serde_json::Value =
        serde_json::from_str(&s).map_err(|e| generic_err(format!("json-parse: {e}")))?;
    Ok(json_to_steel(&value))
}

#[cfg(test)]
mod tests {
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
}
