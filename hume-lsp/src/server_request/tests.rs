use super::*;

#[test]
fn workspace_configuration_answers_null_per_item_with_no_settings() {
    let params =
        serde_json::json!({"items": [{"section": "rust-analyzer"}, {"section": "editor"}]});
    let result = server_request_response("workspace/configuration", &params, None).unwrap();
    assert_eq!(result, serde_json::json!([null, null]));
}

#[test]
fn workspace_configuration_with_no_items_answers_empty_array() {
    let params = serde_json::json!({"items": []});
    let result = server_request_response("workspace/configuration", &params, None).unwrap();
    assert_eq!(result, serde_json::json!([]));
}

#[test]
fn workspace_configuration_resolves_sections_from_settings() {
    let settings = serde_json::json!({"rust-analyzer": {"cargo": {"features": "all"}}});
    let params = serde_json::json!({"items": [
        {"section": "rust-analyzer.cargo.features"},
        {"section": "rust-analyzer"},
        {"section": "nope"},
        {},
    ]});
    let result =
        server_request_response("workspace/configuration", &params, Some(&settings)).unwrap();
    assert_eq!(
        result,
        serde_json::json!([
            "all",
            {"cargo": {"features": "all"}},
            null,
            settings,
        ])
    );
}

// workspace/applyEdit is answered separately (needs `&mut Editor`); see
// hume-editor's `editor::tests::lsp_edits` for its coverage.

#[test]
fn register_and_unregister_capability_and_progress_create_answer_null() {
    for method in [
        "client/registerCapability",
        "client/unregisterCapability",
        "window/workDoneProgress/create",
    ] {
        let result = server_request_response(method, &serde_json::Value::Null, None).unwrap();
        assert_eq!(result, serde_json::Value::Null, "method {method}");
    }
}

#[test]
fn unknown_server_request_is_method_not_found() {
    let err =
        server_request_response("some/madeUpMethod", &serde_json::Value::Null, None).unwrap_err();
    assert_eq!(err.code, -32601);
    assert!(err.message.contains("some/madeUpMethod"));
}

// ── resolve_config_section ────────────────────────────────────────────────

#[test]
fn resolve_config_section_exact_hit() {
    let settings = serde_json::json!({"a": {"b": 1}});
    assert_eq!(
        resolve_config_section(&settings, Some("a")),
        serde_json::json!({"b": 1})
    );
}

#[test]
fn resolve_config_section_nested_dotted_hit() {
    let settings = serde_json::json!({"a": {"b": {"c": 42}}});
    assert_eq!(
        resolve_config_section(&settings, Some("a.b.c")),
        serde_json::json!(42)
    );
}

#[test]
fn resolve_config_section_missing_key_is_null() {
    let settings = serde_json::json!({"a": {"b": 1}});
    assert_eq!(
        resolve_config_section(&settings, Some("a.missing")),
        serde_json::Value::Null
    );
}

#[test]
fn resolve_config_section_non_object_mid_path_is_null() {
    let settings = serde_json::json!({"a": 1});
    assert_eq!(
        resolve_config_section(&settings, Some("a.b")),
        serde_json::Value::Null
    );
}

#[test]
fn resolve_config_section_absent_returns_whole_blob() {
    let settings = serde_json::json!({"a": 1, "b": 2});
    assert_eq!(resolve_config_section(&settings, None), settings);
}

#[test]
fn resolve_config_section_empty_string_returns_whole_blob() {
    let settings = serde_json::json!({"a": 1});
    assert_eq!(resolve_config_section(&settings, Some("")), settings);
}
