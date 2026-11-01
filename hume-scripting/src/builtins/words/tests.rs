use super::*;
use steel::rvals::IntoSteelVal;

/// Extracts the list of strings a successful `split-words` call returns. It
/// walks the list explicitly and assumes nothing about the encoding shape.
fn words(result: SteelResult) -> Vec<String> {
    let SteelVal::ListV(list) = result.expect("split-words must succeed") else {
        panic!("expected a list");
    };
    list.into_iter()
        .map(|v| {
            let SteelVal::StringV(s) = v else {
                panic!("expected a string element, got {v:?}");
            };
            s.to_string()
        })
        .collect()
}

#[test]
fn splits_a_plain_sentence() {
    let got = words(split_words(
        "hello, world!".into_steelval().unwrap(),
        "".into_steelval().unwrap(),
    ));
    assert_eq!(got, vec!["hello", "world"]);
}

#[test]
fn honors_word_chars() {
    let got = words(split_words(
        "foo-bar baz".into_steelval().unwrap(),
        "-".into_steelval().unwrap(),
    ));
    assert_eq!(got, vec!["foo-bar", "baz"]);
}

#[test]
fn does_not_absorb_non_ascii_punctuation() {
    // A `>= 128` word-char approximation would merge these. This must come
    // back as two words, not one merged "l’élément".
    let got = words(split_words(
        "l\u{2019}\u{e9}l\u{e9}ment".into_steelval().unwrap(),
        "".into_steelval().unwrap(),
    ));
    assert_eq!(got, vec!["l", "\u{e9}l\u{e9}ment"]);
}

#[test]
fn empty_line_returns_no_words() {
    let got = words(split_words(
        "".into_steelval().unwrap(),
        "".into_steelval().unwrap(),
    ));
    assert_eq!(got, Vec::<String>::new());
}

#[test]
fn raises_on_non_string_line_argument() {
    let err = split_words(SteelVal::IntV(1), "".into_steelval().unwrap()).unwrap_err();
    assert!(
        err.to_string().contains("line: expected a string"),
        "got: {err}"
    );
}

#[test]
fn raises_on_non_string_word_chars_argument() {
    let err = split_words("foo".into_steelval().unwrap(), SteelVal::IntV(1)).unwrap_err();
    assert!(
        err.to_string().contains("word-chars: expected a string"),
        "got: {err}"
    );
}

/// `word-chars` containing a whitespace character `classify_char` doesn't
/// recognize as `Space` (form feed, not one of the four it special-cases)
/// would silently promote it to `Word` via `WordChars::classify`'s
/// `Punctuation`-only match arm, breaking every run's termination. This
/// must raise instead, same as the settings layer's own `word-chars`
/// validation does at `:set`/`set-option!` time.
#[test]
fn raises_on_word_chars_containing_a_stray_whitespace_char() {
    let err = split_words(
        "foo".into_steelval().unwrap(),
        "\u{c}".into_steelval().unwrap(), // U+000C form feed
    )
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("word-chars: cannot contain whitespace"),
        "got: {err}"
    );
}

fn strings(items: &[&str]) -> SteelVal {
    items
        .iter()
        .map(|s| s.into_steelval().unwrap())
        .collect::<Vec<_>>()
        .into_steelval()
        .unwrap()
}

fn common_prefix_of(items: &[&str]) -> String {
    match string_common_prefix(strings(items)).expect("string-common-prefix must succeed") {
        SteelVal::StringV(s) => s.to_string(),
        other => panic!("expected a string, got {other:?}"),
    }
}

#[test]
fn common_prefix_is_what_every_string_starts_with() {
    assert_eq!(
        common_prefix_of(&["my dir/a", "my dir/b", "my docs/"]),
        "my d"
    );
    assert_eq!(common_prefix_of(&["notes.txt"]), "notes.txt");
    assert_eq!(common_prefix_of(&[]), "");
    assert_eq!(common_prefix_of(&["abc", "xyz"]), "");
}

#[test]
fn common_prefix_never_splits_a_cluster() {
    assert_eq!(common_prefix_of(&["ae\u{301}", "ae\u{300}"]), "a");
}

#[test]
fn common_prefix_raises_on_a_non_string_element() {
    let list = vec!["a".into_steelval().unwrap(), SteelVal::IntV(1)]
        .into_steelval()
        .unwrap();
    let err = string_common_prefix(list).unwrap_err();
    assert!(
        err.to_string().contains("strings: expected a string"),
        "got: {err}"
    );
}

fn width_of(s: &str) -> SteelVal {
    string_width(s.into_steelval().unwrap()).unwrap()
}

#[test]
fn string_width_counts_terminal_cells() {
    assert_eq!(width_of("abc"), SteelVal::IntV(3));
    assert_eq!(width_of("日"), SteelVal::IntV(2));
    assert_eq!(width_of("e\u{301}"), SteelVal::IntV(1));
    assert_eq!(width_of("👩\u{200d}💻"), SteelVal::IntV(2));
    assert_eq!(width_of(""), SteelVal::IntV(0));
}

#[test]
fn string_width_counts_a_tab_as_one_cell_and_a_control_character_as_its_placeholder() {
    assert_eq!(width_of("a\tb"), SteelVal::IntV(3));
    // U+0001 paints as `<1>`, three cells.
    assert_eq!(width_of("a\u{1}"), SteelVal::IntV(4));
}

#[test]
fn string_width_raises_on_a_non_string() {
    let err = string_width(SteelVal::IntV(1)).unwrap_err();
    assert!(
        err.to_string().contains("s: expected a string"),
        "got: {err}"
    );
}

fn fit(s: &str, cells: isize, side: &str) -> String {
    let out = string_fit(
        s.into_steelval().unwrap(),
        SteelVal::IntV(cells),
        SteelVal::SymbolV(side.into()),
    )
    .unwrap();
    let SteelVal::StringV(out) = out else {
        panic!("expected a string, got {out:?}");
    };
    out.to_string()
}

#[test]
fn string_fit_leaves_a_string_that_fits_unchanged() {
    assert_eq!(fit("abcdef", 6, "head"), "abcdef");
    assert_eq!(fit("abcdef", 9, "tail"), "abcdef");
}

#[test]
fn string_fit_cuts_the_named_side_behind_an_ellipsis() {
    insta::assert_snapshot!(fit("src/editor/picker.rs", 10, "head"), @"…picker.rs");
    insta::assert_snapshot!(fit("src/editor/picker.rs", 10, "tail"), @"src/edito…");
}

#[test]
fn string_fit_never_splits_a_cluster() {
    // A ZWJ sequence and a flag each take two cells as one cluster: a
    // one-cell keep drops them whole instead of keeping their last part.
    insta::assert_snapshot!(fit("ab👩\u{200d}💻", 3, "head"), @"…👩‍💻");
    insta::assert_snapshot!(fit("ab👩\u{200d}💻", 2, "head"), @"…");
    insta::assert_snapshot!(fit("x🇮🇹", 2, "head"), @"…");
    insta::assert_snapshot!(fit("🇮🇹x", 2, "tail"), @"…");
}

#[test]
fn string_fit_measures_a_tab_and_a_control_character_as_a_row_paints_them() {
    insta::assert_snapshot!(fit("ab\tcd", 4, "head"), @"…\tcd");
    // `<1>` needs three cells, so a three-cell keep stops before it.
    insta::assert_snapshot!(fit("ab\u{1}cd", 4, "tail"), @"ab…");
}

#[test]
fn string_fit_of_zero_cells_is_empty() {
    assert_eq!(fit("abc", 0, "head"), "");
}

#[test]
fn string_fit_raises_on_a_bad_side_or_a_negative_width() {
    let err = string_fit(
        "abc".into_steelval().unwrap(),
        SteelVal::IntV(2),
        SteelVal::SymbolV("middle".into()),
    )
    .unwrap_err();
    assert!(err.to_string().contains("side"), "got: {err}");
    let err = string_fit(
        "abc".into_steelval().unwrap(),
        SteelVal::IntV(-1),
        SteelVal::SymbolV("head".into()),
    )
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("cells: expected a non-negative integer"),
        "got: {err}"
    );
}
