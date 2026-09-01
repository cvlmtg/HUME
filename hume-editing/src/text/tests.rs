use super::*;

#[test]
fn from_rope_is_raw() {
    // from_rope is the changeset algebra path — it has a debug_assert for
    // the trailing \n but does not add one if missing. The caller
    // (ChangeSet::apply) is responsible for ensuring the invariant holds.
    // The invariant is upheld by From<&str> / empty (user entry points) and
    // by the editing-operation guards (e.g. delete_char_forward is a no-op
    // on the structural \n).
    let rope = Rope::from_str("hello\n");
    let text = BufferText::from_rope(rope, LineEnding::Lf);
    assert_eq!(text.to_string(), "hello\n");
}

#[test]
fn empty_buffer() {
    let text = BufferText::empty();
    assert_eq!(text.len_chars(), 1); // structural trailing \n
    assert_eq!(text.ropey_line_count(), 2); // "\n" → line 0 = "\n", line 1 = ""
    assert!(text.is_empty());
    assert_eq!(text.to_string(), "\n");
}

#[test]
fn from_str_ascii() {
    let text = BufferText::from("hello\nworld");
    assert_eq!(text.len_chars(), 12); // "hello\nworld\n"
    assert_eq!(text.ropey_line_count(), 3); // line 0, line 1, trailing empty line
    assert!(!text.is_empty());
    assert_eq!(text.to_string(), "hello\nworld\n");
}

#[test]
fn from_str_lf_line_ending() {
    let text = BufferText::from("hello\n");
    assert_eq!(text.line_ending(), LineEnding::Lf);
}

#[test]
fn from_str_crlf_normalized() {
    let text = BufferText::from("hello\r\nworld\r\n");
    // \r stripped — content is pure LF
    assert_eq!(text.to_string(), "hello\nworld\n");
    assert_eq!(text.len_chars(), 12); // "hello\nworld\n"
    assert_eq!(text.line_ending(), LineEnding::CrLf);
}

#[test]
fn from_str_mixed_crlf_lf() {
    // Mixed: CRLF wins if any \r\n present.
    let text = BufferText::from("hello\r\nworld\n");
    assert_eq!(text.to_string(), "hello\nworld\n");
    assert_eq!(text.line_ending(), LineEnding::CrLf);
}

#[test]
fn from_str_bare_cr_normalized() {
    // Old Mac bare \r normalizes to \n, same as a \r\n pair.
    let text = BufferText::from("hello\rworld\n");
    assert_eq!(text.to_string(), "hello\nworld\n");
    assert_eq!(text.line_ending(), LineEnding::Lf);
}

#[test]
fn from_str_cr_then_crlf_fully_normalizes() {
    // "\r\r\n": the first '\r' (not itself followed by '\n' — its lookahead
    // is the second '\r') normalizes to '\n' on its own; the second '\r'
    // pairs with the following '\n' and normalizes to a single '\n' too.
    // Two real line breaks in, two '\n's out — no literal '\r' survives.
    let text = BufferText::from("\r\r\n");
    assert_eq!(text.to_string(), "\n\n");
    // Detection only looks at the original input for a \r\n pair, which is
    // present (the second '\r' + the '\n'), so this still reads as CrLf.
    assert_eq!(text.line_ending(), LineEnding::CrLf);
}

// ── normalize_line_endings ───────────────────────────────────────────────

#[test]
fn normalize_line_endings_crlf() {
    assert_eq!(normalize_line_endings("a\r\nb"), "a\nb");
}

#[test]
fn normalize_line_endings_lone_cr() {
    assert_eq!(normalize_line_endings("a\rb"), "a\nb");
}

#[test]
fn normalize_line_endings_is_noop_on_lf() {
    assert_eq!(normalize_line_endings("a\nb"), "a\nb");
}

#[test]
fn from_str_trailing_newline() {
    // A trailing newline creates an extra empty line.
    let text = BufferText::from("hello\n");
    assert_eq!(text.ropey_line_count(), 2);
}

#[test]
fn line_tokens_count_matches_ropey_line_count_and_keeps_terminators() {
    let text = BufferText::from("a\nb\n");
    let tokens: Vec<_> = text.line_tokens().collect();
    assert_eq!(tokens.len(), text.ropey_line_count());
    assert_eq!(tokens, vec!["a\n", "b\n", ""]);
}

#[test]
fn line_tokens_treats_every_non_lf_break_char_as_content() {
    // `\n` is the only line break under this workspace's ropey config, so a
    // form feed doesn't split a token. Nor would a `\r`, which additionally
    // can't reach a live buffer at all — `BufferText::from` normalizes it to
    // `\n`, so it becomes a real break by being rewritten, not by ropey
    // recognizing it.
    let text = BufferText::from("a\u{0C}b\n");
    assert_eq!(
        text.line_tokens().collect::<Vec<_>>(),
        vec!["a\u{0C}b\n", ""]
    );

    let cr = BufferText::from("a\rb\n");
    assert_eq!(cr.to_string(), "a\nb\n");
    assert_eq!(cr.line_tokens().collect::<Vec<_>>(), vec!["a\n", "b\n", ""]);
}

#[test]
fn line_tokens_at_starts_at_the_requested_line() {
    let text = BufferText::from("a\nb\nc\n");
    let tokens: Vec<_> = text.line_tokens_at(1).collect();
    assert_eq!(tokens, vec!["b\n", "c\n", ""]);
}

#[test]
fn content_line_count_excludes_the_phantom_trailing_line() {
    assert_eq!(BufferText::from("\n").content_line_count(), 1);
    assert_eq!(BufferText::from("a\nb\nc\n").content_line_count(), 3);
}

#[test]
fn last_content_line_is_content_line_count_minus_one() {
    assert_eq!(BufferText::from("\n").last_content_line(), 0);
    assert_eq!(BufferText::from("a\nb\nc\n").last_content_line(), 2);
}

#[test]
fn from_str_unicode() {
    // "é" can be represented as a single char (U+00E9) or as two chars
    // (U+0065 + U+0301 combining accent). `BufferText::from` accepts whatever
    // Rust gives us. Here we use the precomposed form — one char.
    let text = BufferText::from("café");
    assert_eq!(text.len_chars(), 5); // c a f é \n
}

#[test]
fn line_to_char() {
    let text = BufferText::from("hello\nworld\nfoo");
    assert_eq!(text.line_to_char(0), 0); // "hello" starts at 0
    assert_eq!(text.line_to_char(1), 6); // "world" starts after "hello\n"
    assert_eq!(text.line_to_char(2), 12); // "foo" starts after "world\n"
}

#[test]
fn char_to_line() {
    let text = BufferText::from("hello\nworld\nfoo");
    assert_eq!(text.char_to_line(0), 0); // 'h' is on line 0
    assert_eq!(text.char_to_line(5), 0); // '\n' is still line 0
    assert_eq!(text.char_to_line(6), 1); // 'w' is on line 1
    assert_eq!(text.char_to_line(12), 2); // 'f' is on line 2
}

#[test]
fn insert_at_start() {
    let text = BufferText::from("world");
    let new = text.insert(0, "hello ");
    assert_eq!(new.to_string(), "hello world\n");
    // Original is unchanged — structural sharing.
    assert_eq!(text.to_string(), "world\n");
}

#[test]
fn insert_at_end() {
    // Insert before the trailing \n (position 5 in "hello\n").
    let text = BufferText::from("hello");
    let new = text.insert(5, " world");
    assert_eq!(new.to_string(), "hello world\n");
}

#[test]
fn insert_in_middle() {
    let text = BufferText::from("helo");
    let new = text.insert(3, "l"); // "hel" + "l" + "o\n"
    assert_eq!(new.to_string(), "hello\n");
}

#[test]
fn remove_whole() {
    let text = BufferText::from("hello");
    let new = text.remove(0..5); // removes "hello", leaving "\n"
    assert_eq!(new.to_string(), "\n");
    assert!(new.is_empty());
    assert_eq!(text.to_string(), "hello\n"); // original unchanged
}

#[test]
fn remove_range() {
    let text = BufferText::from("hello world");
    let new = text.remove(5..11); // remove " world"
    assert_eq!(new.to_string(), "hello\n");
}

#[test]
fn insert_then_remove_is_identity() {
    let original = BufferText::from("hello world");
    let after_insert = original.insert(5, " beautiful");
    let restored = after_insert.remove(5..15);
    assert_eq!(restored, original);
}

#[test]
fn slice() {
    let text = BufferText::from("hello world");
    let s: String = text.slice(6..11).to_string();
    assert_eq!(s, "world");
}

#[test]
fn equality() {
    let a = BufferText::from("hello");
    let b = BufferText::from("hello");
    let c = BufferText::from("world");
    assert_eq!(a, b);
    assert_ne!(a, c);
}

// ── char_at boundary cases ────────────────────────────────────────────────

#[test]
fn char_at_first_position() {
    let text = BufferText::from("hello");
    assert_eq!(text.char_at(0), Some('h'));
}

#[test]
fn char_at_last_position() {
    // "hello" + structural '\n' → last char is '\n' at len_chars()-1.
    let text = BufferText::from("hello");
    assert_eq!(text.char_at(text.len_chars() - 1), Some('\n'));
}

#[test]
fn char_at_out_of_bounds() {
    let text = BufferText::from("hello");
    assert_eq!(text.char_at(text.len_chars()), None);
}

// ── single-char buffer ────────────────────────────────────────────────────

#[test]
fn single_char_buffer_has_two_chars() {
    // "x" gets the structural '\n' appended → len_chars() == 2.
    let text = BufferText::from("x");
    assert_eq!(text.len_chars(), 2);
    assert!(!text.is_empty());
    assert_eq!(text.char_at(0), Some('x'));
    assert_eq!(text.char_at(1), Some('\n'));
}

// ── remove with empty range ───────────────────────────────────────────────

#[test]
fn remove_empty_range_is_identity() {
    let text = BufferText::from("hello");
    let same = text.remove(3..3);
    assert_eq!(same.to_string(), "hello\n");
}

// ── insert/remove with multi-byte content ─────────────────────────────────

#[test]
fn insert_grapheme_cluster() {
    // Insert a two-char grapheme (e + combining acute) at position 0.
    let text = BufferText::from("hello");
    let new = text.insert(0, "e\u{0301}");
    // 'e' + U+0301 + "hello" + '\n' = 8 chars.
    assert_eq!(new.len_chars(), 8);
    assert_eq!(new.char_at(0), Some('e'));
    assert_eq!(new.char_at(1), Some('\u{0301}'));
    assert_eq!(new.char_at(2), Some('h'));
}

#[test]
fn remove_grapheme_cluster_range() {
    // Remove a two-char grapheme cluster.
    let text = BufferText::from("e\u{0301}hello");
    // text: 'e'(0) U+0301(1) 'h'(2) 'e'(3) ... '\n'(7) = 8 chars.
    let new = text.remove(0..2); // remove the 'e' + combining accent
    assert_eq!(new.to_string(), "hello\n");
}
