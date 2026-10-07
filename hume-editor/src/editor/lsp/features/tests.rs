use super::*;
use serde_json::json;

#[test]
fn advertises_requires_a_truthy_provider_key() {
    assert!(advertises(
        LspFeature::Hover,
        &json!({ "hoverProvider": true })
    ));
    assert!(advertises(
        LspFeature::Hover,
        &json!({ "hoverProvider": {} })
    ));
    assert!(!advertises(
        LspFeature::Hover,
        &json!({ "hoverProvider": false })
    ));
    assert!(!advertises(
        LspFeature::Hover,
        &json!({ "hoverProvider": null })
    ));
    assert!(!advertises(LspFeature::Hover, &json!({})));
}

#[test]
fn diagnostics_need_no_capability() {
    assert!(advertises(LspFeature::Diagnostics, &json!({})));
}

#[test]
fn format_accepts_either_formatting_provider() {
    assert!(advertises(
        LspFeature::Format,
        &json!({ "documentFormattingProvider": true })
    ));
    assert!(advertises(
        LspFeature::Format,
        &json!({ "documentRangeFormattingProvider": true })
    ));
    assert!(!advertises(
        LspFeature::Format,
        &json!({ "hoverProvider": true })
    ));
}

#[test]
fn requirement_names_the_feature_of_a_standard_method() {
    let hover = requirement("textDocument/hover").expect("hover is a standard method");
    assert_eq!(hover.feature, LspFeature::Hover);
    assert_eq!(hover.capability, None);
}

#[test]
fn formatting_methods_name_their_own_capability() {
    let whole = requirement("textDocument/formatting").unwrap();
    let range = requirement("textDocument/rangeFormatting").unwrap();
    let ranges = requirement("textDocument/rangesFormatting").unwrap();
    assert_eq!(whole.feature, LspFeature::Format);
    assert_eq!(whole.capability, Some(&["documentFormattingProvider"][..]));
    assert_eq!(
        range.capability,
        Some(&["documentRangeFormattingProvider"][..])
    );
    assert_eq!(ranges.capability, range.capability);
}

#[test]
fn resolve_methods_need_their_features_resolve_provider() {
    let completion = requirement("completionItem/resolve").unwrap();
    let action = requirement("codeAction/resolve").unwrap();
    assert_eq!(completion.feature, LspFeature::Completion);
    assert_eq!(
        completion.capability,
        Some(&["completionProvider", "resolveProvider"][..])
    );
    assert_eq!(action.feature, LspFeature::CodeAction);
    assert_eq!(
        action.capability,
        Some(&["codeActionProvider", "resolveProvider"][..])
    );
}

#[test]
fn capability_at_follows_a_path_and_rejects_false_and_null() {
    let caps = json!({
        "a": { "b": true, "c": false, "d": null, "e": {} },
        "f": false,
    });
    assert_eq!(capability_at(&caps, &["a", "b"]), Some(&json!(true)));
    assert_eq!(capability_at(&caps, &["a", "e"]), Some(&json!({})));
    assert_eq!(capability_at(&caps, &["a", "c"]), None);
    assert_eq!(capability_at(&caps, &["a", "d"]), None);
    assert_eq!(capability_at(&caps, &["a", "missing"]), None);
    assert_eq!(capability_at(&caps, &["f", "b"]), None);
}

#[test]
fn provider_of_a_resolve_method_is_the_nested_flag() {
    let with = json!({ "completionProvider": { "resolveProvider": true } });
    let without = json!({ "completionProvider": { "triggerCharacters": ["."] } });
    let query = CapabilityQuery::Method("completionItem/resolve");
    assert_eq!(provider(&with, query), Some(&json!(true)));
    assert_eq!(provider(&without, query), None);
}

#[test]
fn custom_and_follow_up_methods_have_no_requirement() {
    assert!(requirement("custom/ping").is_none());
    assert!(requirement("workspace/executeCommand").is_none());
}

#[test]
fn a_formatting_method_needs_a_key_its_feature_advertises_by() {
    for method in [
        "textDocument/formatting",
        "textDocument/rangeFormatting",
        "textDocument/rangesFormatting",
    ] {
        let needed = requirement(method).unwrap().capability.unwrap()[0];
        assert!(
            capability_keys(LspFeature::Format).contains(&needed),
            "{method} needs {needed}, which Format does not advertise by"
        );
    }
}
