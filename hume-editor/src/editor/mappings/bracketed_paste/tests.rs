use super::flatten_for_minibuf;

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
