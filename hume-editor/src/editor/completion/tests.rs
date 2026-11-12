use std::ops::Range;

use super::MinibufArg;

/// The argument's prefix and the span completing all of it replaces.
fn whole(input: &str, floor: usize, cursor: usize) -> (&str, Range<usize>) {
    let arg = MinibufArg::new(input, floor, cursor);
    (arg.prefix(), arg.token_span(&[' ']))
}

#[test]
fn minibuf_arg_starts_past_the_whitespace_after_the_floor() {
    assert_eq!(whole("e  foo", 1, 6), ("foo", 3..6));
}

#[test]
fn minibuf_arg_starts_at_the_floor_with_no_whitespace() {
    assert_eq!(whole("e!foo", 2, 5), ("foo", 2..5));
}

#[test]
fn minibuf_arg_is_empty_at_a_cursor_inside_the_leading_whitespace() {
    assert_eq!(whole("e   foo", 1, 2), ("", 2..2));
}

#[test]
fn minibuf_arg_with_floor_zero_is_the_whole_line() {
    assert_eq!(whole(" foo", 0, 4), ("foo", 1..4));
}

#[test]
fn minibuf_arg_last_word_span_is_the_word_before_the_cursor() {
    assert_eq!(
        MinibufArg::new("set global th", 3, 13).last_word_span(),
        11..13
    );
}

#[test]
fn minibuf_arg_last_word_span_starts_at_a_no_space_argument() {
    assert_eq!(MinibufArg::new("b1", 1, 2).last_word_span(), 1..2);
}

#[test]
fn minibuf_arg_token_span_runs_past_a_cursor_inside_the_token() {
    assert_eq!(whole("e src/main.rs", 1, 8), ("src/ma", 2..13));
}

#[test]
#[should_panic(expected = "floor")]
fn minibuf_arg_refuses_a_floor_past_the_cursor() {
    MinibufArg::new("e foo", 3, 1);
}
