use super::*;

fn compiled(pattern: &str) -> ArgRegex {
    let re = regex(pattern.into_steelval().unwrap()).expect("regex must compile");
    ArgRegex::from_steelval(&re).expect("regex returns a regex value")
}

/// The `(start . end)` pairs `string-matches` returns for `pattern` on `text`.
fn matches(text: &str, pattern: &str) -> Vec<(isize, isize)> {
    let SteelVal::ListV(list) =
        string_matches(text.to_string(), compiled(pattern)).expect("string-matches must succeed")
    else {
        panic!("expected a list");
    };
    list.into_iter()
        .map(|v| {
            let SteelVal::Pair(pair) = &v else {
                panic!("expected a pair, got {v:?}");
            };
            match (pair.car(), pair.cdr()) {
                (SteelVal::IntV(start), SteelVal::IntV(end)) => (start, end),
                other => panic!("expected two integers, got {other:?}"),
            }
        })
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
    let re = regex("v/a.b".into_steelval().unwrap()).unwrap();
    assert_eq!(re.to_string(), "#<regex \"v/a.b\">");
}

#[test]
fn an_invalid_or_empty_pattern_raises() {
    for pattern in ["(", "", "v/"] {
        let err = regex(pattern.into_steelval().unwrap()).unwrap_err();
        assert!(
            err.to_string()
                .contains(&format!("{pattern:?} is not a valid search pattern")),
            "got: {err}"
        );
    }
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
    let err = ArgRegex::from_steelval(&"ab".into_steelval().unwrap())
        .err()
        .unwrap();
    assert!(
        err.to_string().contains("expected a regex, got"),
        "got: {err}"
    );
}
