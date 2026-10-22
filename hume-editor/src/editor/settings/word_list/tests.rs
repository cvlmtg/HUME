use super::*;

fn words(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn parse_splits_on_any_whitespace() {
    assert_eq!(
        parse_word_list("  sh \t -c ").unwrap(),
        words(&["sh", "-c"])
    );
}

#[test]
fn parse_groups_a_quoted_word_with_spaces() {
    assert_eq!(
        parse_word_list(r#""C:\Program Files\Git\bin\bash.exe" -c"#).unwrap(),
        words(&[r"C:\Program Files\Git\bin\bash.exe", "-c"])
    );
}

#[test]
fn parse_unescapes_a_quote_and_a_backslash_inside_quotes() {
    assert_eq!(
        parse_word_list(r#""a \"b\" \\c""#).unwrap(),
        words(&[r#"a "b" \c"#])
    );
}

#[test]
fn parse_keeps_a_backslash_outside_quotes() {
    assert_eq!(
        parse_word_list(r"C:\bin\sh").unwrap(),
        words(&[r"C:\bin\sh"])
    );
}

#[test]
fn parse_joins_a_quoted_segment_to_the_word_around_it() {
    assert_eq!(parse_word_list(r#"a"b c"d"#).unwrap(), words(&["ab cd"]));
}

#[test]
fn parse_keeps_an_empty_quoted_word() {
    assert_eq!(parse_word_list(r#"x """#).unwrap(), words(&["x", ""]));
}

#[test]
fn parse_refuses_no_words() {
    assert!(parse_word_list("").is_err());
    assert!(parse_word_list("   ").is_err());
}

#[test]
fn parse_refuses_an_unterminated_quote() {
    assert!(parse_word_list(r#"sh "-c"#).is_err());
}

#[test]
fn format_round_trips_through_parse() {
    let cases = [
        words(&["sh", "-c"]),
        words(&[r"C:\Program Files\sh.exe", "-c"]),
        words(&[r#"say "hi""#, r"back\slash", r"C:\bin"]),
        words(&["", "x"]),
        words(&["tab\there"]),
    ];
    for case in cases {
        let text = format_word_list(&case);
        assert_eq!(parse_word_list(&text).unwrap(), case, "via {text:?}");
    }
}

#[test]
fn format_leaves_plain_words_bare() {
    assert_eq!(format_word_list(&words(&["bash", "-lc"])), "bash -lc");
}
