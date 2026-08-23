use super::*;

fn location(uri: &str, line: u64, character: u64) -> serde_json::Value {
    serde_json::json!({
        "uri": uri,
        "range": {"start": {"line": line, "character": character},
                   "end": {"line": line, "character": character}}
    })
}

fn location_link(uri: &str, line: u64, character: u64) -> serde_json::Value {
    serde_json::json!({
        "targetUri": uri,
        "targetRange": {"start": {"line": line, "character": character},
                         "end": {"line": line, "character": character + 1}},
        "targetSelectionRange": {"start": {"line": line, "character": character},
                                  "end": {"line": line, "character": character + 1}}
    })
}

#[test]
fn decodes_a_plain_location() {
    let wl = decode_location(&location("file:///tmp/a.rs", 3, 7), "test").expect("decode");
    assert_eq!(wl.uri.as_str(), "file:///tmp/a.rs");
    assert_eq!(wl.line, 3);
    assert_eq!(wl.character, 7);
}

/// `targetSelectionRange` wins over `targetRange` when both are present —
/// the narrower, symbol-only span a client should land on, per the LSP
/// spec's own guidance for `LocationLink`.
#[test]
fn decodes_a_location_link_preferring_target_selection_range() {
    let mut link = location_link("file:///tmp/b.rs", 5, 2);
    link["targetSelectionRange"] = serde_json::json!({"start": {"line": 9, "character": 1}, "end": {"line": 9, "character": 2}});
    let wl = decode_location(&link, "test").expect("decode");
    assert_eq!(wl.line, 9);
    assert_eq!(wl.character, 1);
}

/// A `LocationLink` with no `targetSelectionRange` at all — only some
/// servers send it — must fall back to `targetRange` rather than error.
#[test]
fn decodes_a_location_link_with_only_target_range() {
    let mut link = location_link("file:///tmp/c.rs", 4, 0);
    link.as_object_mut().unwrap().remove("targetSelectionRange");
    let wl = decode_location(&link, "test").expect("decode");
    assert_eq!(wl.line, 4);
    assert_eq!(wl.character, 0);
}

#[test]
fn missing_uri_errors_naming_the_caller() {
    let loc = serde_json::json!({"range": {"start": {"line": 0, "character": 0}}});
    let err = decode_location(&loc, "my-caller").unwrap_err();
    assert_eq!(err, "my-caller: missing uri");
}

#[test]
fn missing_range_errors() {
    let loc = serde_json::json!({"uri": "file:///tmp/a.rs"});
    let err = decode_location(&loc, "test").unwrap_err();
    assert_eq!(err, "test: missing range");
}

#[test]
fn missing_range_start_line_errors() {
    let loc = serde_json::json!({
        "uri": "file:///tmp/a.rs",
        "range": {"start": {"character": 0}}
    });
    let err = decode_location(&loc, "test").unwrap_err();
    assert_eq!(err, "test: missing range.start.line");
}

#[test]
fn missing_range_start_character_errors() {
    let loc = serde_json::json!({
        "uri": "file:///tmp/a.rs",
        "range": {"start": {"line": 0}}
    });
    let err = decode_location(&loc, "test").unwrap_err();
    assert_eq!(err, "test: missing range.start.character");
}

#[test]
fn non_integer_line_errors_as_missing() {
    let loc = serde_json::json!({
        "uri": "file:///tmp/a.rs",
        "range": {"start": {"line": "not-a-number", "character": 0}}
    });
    let err = decode_location(&loc, "test").unwrap_err();
    assert_eq!(err, "test: missing range.start.line");
}

#[test]
fn unparseable_uri_errors() {
    // A `:` with no scheme name before it is not a valid URI at all.
    let loc = location(":::not a uri:::", 0, 0);
    let err = decode_location(&loc, "test").unwrap_err();
    assert!(
        err.starts_with("test: bad uri"),
        "expected a bad-uri error, got {err:?}"
    );
}

/// A location link whose `targetUri` is absent entirely (neither shape
/// matches) must report the same "missing uri" as a plain `Location`.
#[test]
fn location_link_missing_target_uri_errors_as_missing_uri() {
    let loc = serde_json::json!({
        "targetRange": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}}
    });
    let err = decode_location(&loc, "test").unwrap_err();
    assert_eq!(err, "test: missing uri");
}
