use super::*;
use crate::builtins::args::{list_items, pair_fields};
use crate::testing::test_support::str_val;

fn compiled(pattern: &str) -> ArgRegex {
    let re = regex(str_val(pattern)).expect("regex must compile");
    ArgRegex::from_steelval(&re).expect("regex returns a regex value")
}

/// The `(start . end)` pairs `string-matches` returns for `pattern` on `text`.
fn matches(text: &str, pattern: &str) -> Vec<(isize, isize)> {
    let list =
        string_matches(str_val(text), compiled(pattern)).expect("string-matches must succeed");
    list_items(list, "string-matches")
        .expect("expected a list")
        .into_iter()
        .map(
            |v| match pair_fields(v, "string-matches", "a pair").expect("expected a pair") {
                (SteelVal::IntV(start), SteelVal::IntV(end)) => (start, end),
                other => panic!("expected two integers, got {other:?}"),
            },
        )
        .collect()
}

#[test]
fn every_match_in_order() {
    assert_eq!(matches("ab ab xab", "ab"), [(0, 2), (3, 5), (7, 9)]);
}

#[test]
fn no_match_is_an_empty_list() {
    assert_eq!(matches("abc", "z"), []);
}

#[test]
fn matches_never_overlap() {
    assert_eq!(matches("aaaa", "aa"), [(0, 2), (2, 4)]);
}

#[test]
fn offsets_count_chars_not_bytes() {
    assert_eq!(
        matches("\u{e9}#fff \u{1f600}#000", "#[0-9a-f]+"),
        [(1, 5), (7, 11)]
    );
}

#[test]
fn a_match_widens_to_whole_clusters() {
    assert_eq!(matches("ae\u{301}b", "\u{301}"), [(1, 3)]);
}

#[test]
fn a_match_ends_at_the_end_of_the_string() {
    assert_eq!(matches("a ", "\\s+"), [(1, 2)]);
    assert_eq!(matches("foo bar", "bar$"), [(4, 7)]);
    assert_eq!(matches("ab", "\\n"), []);
}

#[test]
fn offsets_count_the_line_breaks_the_string_holds() {
    assert_eq!(matches("a\r\nb", "b"), [(3, 4)]);
    assert_eq!(matches("a\rb", "\\r"), [(1, 2)]);
}

#[test]
fn zero_width_matches_are_skipped() {
    assert_eq!(matches("ab", "x*"), []);
}

#[test]
fn a_lowercase_pattern_ignores_case() {
    assert_eq!(matches("ABC abc", "abc"), [(0, 3), (4, 7)]);
    assert_eq!(matches("ABC Abc", "Abc"), [(4, 7)]);
}

#[test]
fn the_verbatim_flag_matches_literally() {
    assert_eq!(matches("axb a.b", "v/a.b"), [(4, 7)]);
}

#[test]
fn a_regex_prints_its_pattern() {
    let re = regex(str_val("v/a.b")).unwrap();
    assert_eq!(re.to_string(), "#<regex \"v/a.b\">");
}

#[test]
fn an_invalid_or_empty_pattern_raises() {
    for pattern in ["(", "", "v/"] {
        let err = regex(str_val(pattern)).unwrap_err();
        assert!(
            err.to_string()
                .contains(&format!("{pattern:?} is not a valid search pattern")),
            "got: {err}"
        );
    }
}

#[test]
fn the_multi_flag_changes_nothing() {
    assert_eq!(matches("foo m/foo", "m/foo"), [(0, 3), (6, 9)]);
    assert_eq!(matches("a.b axb", "mv/a.b"), [(0, 3)]);
}

#[test]
fn a_non_string_pattern_raises() {
    let err = regex(SteelVal::IntV(1)).unwrap_err();
    assert!(
        err.to_string().contains("pattern: expected a string"),
        "got: {err}"
    );
}

#[test]
fn a_value_that_is_not_a_regex_is_refused() {
    let err = ArgRegex::from_steelval(&str_val("ab")).err().unwrap();
    assert!(
        err.to_string().contains("expected a regex, got"),
        "got: {err}"
    );
}
