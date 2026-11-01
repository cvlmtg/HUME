use steel::rvals::SteelVal;

use super::row_display;
use crate::builtins::args::{list_of, symbol_hash};
use crate::host::{RowDisplay, RowSpanScopes};
use hume_rope::spanned::SpannedText;

fn string(s: &str) -> SteelVal {
    SteelVal::StringV(s.into())
}

fn span(start: isize, end: isize, scope: SteelVal) -> SteelVal {
    symbol_hash([
        ("start-char", SteelVal::IntV(start)),
        ("end-char", SteelVal::IntV(end)),
        ("scope", scope),
    ])
}

fn styled(text: &str, spans: Vec<SteelVal>) -> SteelVal {
    symbol_hash([("text", string(text)), ("spans", list_of(spans))])
}

fn decode_err(val: SteelVal) -> String {
    row_display(val, "row")
        .expect_err("the display must not decode")
        .to_string()
}

#[test]
fn a_string_display_has_no_spans() {
    assert_eq!(
        row_display(string("a"), "row").unwrap(),
        RowDisplay {
            content: SpannedText::plain("a".to_string()),
            selected_scope: None,
        }
    );
}

#[test]
fn a_symbol_display_reads_as_its_name() {
    assert_eq!(
        row_display(SteelVal::SymbolV("abc".into()), "row").unwrap(),
        RowDisplay {
            content: SpannedText::plain("abc".to_string()),
            selected_scope: None,
        }
    );
}

#[test]
fn a_styled_display_decodes_text_and_spans() {
    let val = styled("abc", vec![span(1, 3, string("markup.italic"))]);
    assert_eq!(
        row_display(val, "row").unwrap(),
        RowDisplay {
            content: SpannedText::new(
                "abc".to_string(),
                vec![(
                    1,
                    3,
                    RowSpanScopes {
                        scope: "markup.italic".to_string(),
                        selected: None,
                    }
                )],
            )
            .unwrap(),
            selected_scope: None,
        }
    );
}

#[test]
fn a_span_decodes_its_selected_scope() {
    let span = symbol_hash([
        ("start-char", SteelVal::IntV(0)),
        ("end-char", SteelVal::IntV(1)),
        ("scope", string("markup.italic")),
        ("selected-scope", SteelVal::SymbolV("markup.bold".into())),
    ]);
    assert_eq!(
        row_display(styled("abc", vec![span]), "row").unwrap(),
        RowDisplay {
            content: SpannedText::new(
                "abc".to_string(),
                vec![(
                    0,
                    1,
                    RowSpanScopes {
                        scope: "markup.italic".to_string(),
                        selected: Some("markup.bold".to_string()),
                    }
                )],
            )
            .unwrap(),
            selected_scope: None,
        }
    );
}

#[test]
fn a_styled_display_decodes_its_selected_scope() {
    let val = symbol_hash([
        ("text", string("abc")),
        ("spans", list_of([])),
        ("selected-scope", string("diagnostic.warning.message")),
    ]);
    assert_eq!(
        row_display(val, "row").unwrap(),
        RowDisplay {
            content: SpannedText::plain("abc".to_string()),
            selected_scope: Some("diagnostic.warning.message".to_string()),
        }
    );
}

#[test]
fn an_unknown_key_in_a_row_or_a_span_raises() {
    let row = symbol_hash([
        ("text", string("abc")),
        ("spans", list_of([])),
        ("bogus", SteelVal::IntV(1)),
    ]);
    let in_span = styled(
        "abc",
        vec![symbol_hash([
            ("start-char", SteelVal::IntV(0)),
            ("end-char", SteelVal::IntV(1)),
            ("scope", string("keyword")),
            ("bogus", SteelVal::IntV(1)),
        ])],
    );
    for val in [row, in_span] {
        let err = decode_err(val);
        assert!(err.contains("unknown key 'bogus"), "got: {err}");
    }
}

#[test]
fn a_span_that_does_not_fit_the_text_raises_naming_its_place() {
    let err = decode_err(styled(
        "abc",
        vec![span(0, 1, string("a")), span(2, 9, string("b"))],
    ));
    assert_eq!(
        err.lines().next().unwrap_or_default(),
        "Error: Generic: row 'spans 1: end 9 is past the text's char length 3"
    );
}

#[test]
fn a_non_string_scope_raises() {
    let err = decode_err(styled("abc", vec![span(0, 1, SteelVal::IntV(5))]));
    assert!(err.contains("expected a string"), "got: {err}");
}

#[test]
fn a_styled_display_without_spans_has_none() {
    let val = symbol_hash([
        ("text", string("abc")),
        ("selected-scope", string("diagnostic.warning.message")),
    ]);
    assert_eq!(
        row_display(val, "row").unwrap(),
        RowDisplay {
            content: SpannedText::plain("abc".to_string()),
            selected_scope: Some("diagnostic.warning.message".to_string()),
        }
    );
}

#[test]
fn a_display_that_is_neither_text_nor_a_hash_names_both_forms() {
    let err = decode_err(list_of([string("abc")]));
    assert!(
        err.contains("expected a string or (hash 'text t ...)"),
        "got: {err}"
    );
}
