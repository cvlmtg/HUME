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
    assert!(err.to_string().contains("split-words line"), "got: {err}");
}

#[test]
fn raises_on_non_string_word_chars_argument() {
    let err = split_words("foo".into_steelval().unwrap(), SteelVal::IntV(1)).unwrap_err();
    assert!(
        err.to_string().contains("split-words word-chars"),
        "got: {err}"
    );
}

/// `word-chars` containing a whitespace character `classify_char` doesn't
/// recognize as `Space` (form feed, not one of the four it special-cases)
/// would silently promote it to `Word` via `WordChars::classify`'s
/// `Punctuation`-only match arm, breaking every run's termination — this
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
        err.to_string().contains("split-words word-chars"),
        "got: {err}"
    );
}
