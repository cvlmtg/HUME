use super::{flatten_for_minibuf, normalize_paste_newlines};

#[test]
fn normalizes_crlf() {
    assert_eq!(normalize_paste_newlines("a\r\nb"), "a\nb");
}

#[test]
fn normalizes_lone_cr() {
    assert_eq!(normalize_paste_newlines("a\rb"), "a\nb");
}

#[test]
fn normalize_is_noop_on_lf() {
    assert_eq!(normalize_paste_newlines("a\nb"), "a\nb");
}

#[test]
fn flatten_drops_trailing_newline() {
    assert_eq!(flatten_for_minibuf("foo\n"), "foo");
}

#[test]
fn flatten_turns_interior_newline_into_space() {
    assert_eq!(flatten_for_minibuf("foo\nbar"), "foo bar");
}

#[test]
fn flatten_drops_multiple_trailing_newlines() {
    assert_eq!(flatten_for_minibuf("foo\n\n"), "foo");
}
