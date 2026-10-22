use super::*;

fn diagnostic_wire(message: &str) -> serde_json::Value {
    serde_json::json!({
        "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
        "message": message,
    })
}

#[test]
fn unchanged_report_carries_only_its_result_id() {
    let report = serde_json::json!({"kind": "unchanged", "resultId": "7"});
    let Ok(PullReport::Unchanged { result_id }) = parse_pull_report(report) else {
        panic!("an unchanged report");
    };
    assert_eq!(result_id.as_deref(), Some("7"));
}

#[test]
fn full_report_parses_each_item_and_skips_the_ones_that_fail() {
    let report = serde_json::json!({
        "kind": "full",
        "items": [diagnostic_wire("first"), {"message": "no range"}, diagnostic_wire("last")],
    });
    let Ok(PullReport::Full {
        result_id,
        diagnostics,
        skipped,
    }) = parse_pull_report(report)
    else {
        panic!("a full report");
    };
    assert_eq!(result_id, None);
    let messages: Vec<_> = diagnostics
        .iter()
        .map(|d| d.diagnostic.message.as_str())
        .collect();
    assert_eq!(messages, ["first", "last"]);
    assert_eq!(diagnostics[0].raw, diagnostic_wire("first"));
    assert_eq!(skipped.len(), 1);
}

#[test]
fn full_report_without_items_is_refused() {
    let error = parse_pull_report(serde_json::json!({"kind": "full"})).unwrap_err();
    assert_eq!(error, "a diagnostics report without items");
}

#[test]
fn full_report_none_of_whose_items_parse_is_refused() {
    let report = serde_json::json!({"kind": "full", "items": [{"message": "no range"}]});
    let error = parse_pull_report(report).unwrap_err();
    assert_eq!(error, "a diagnostics report none of whose items parse");
}

#[test]
fn report_of_an_unknown_kind_is_refused() {
    let error = parse_pull_report(serde_json::json!({"kind": "partial"})).unwrap_err();
    assert_eq!(error, "a diagnostics report of an unknown kind");
}
