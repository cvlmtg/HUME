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
    let buf = Text::from_rope(rope, LineEnding::Lf);
    assert_eq!(buf.to_string(), "hello\n");
}

#[test]
fn empty_buffer() {
    let buf = Text::empty();
    assert_eq!(buf.len_chars(), 1); // structural trailing \n
    assert_eq!(buf.len_lines(), 2); // "\n" → line 0 = "\n", line 1 = ""
    assert!(buf.is_empty());
    assert_eq!(buf.to_string(), "\n");
}

#[test]
fn from_str_ascii() {
    let buf = Text::from("hello\nworld");
    assert_eq!(buf.len_chars(), 12); // "hello\nworld\n"
    assert_eq!(buf.len_lines(), 3); // line 0, line 1, trailing empty line
    assert!(!buf.is_empty());
    assert_eq!(buf.to_string(), "hello\nworld\n");
}

#[test]
fn from_str_lf_line_ending() {
    let buf = Text::from("hello\n");
    assert_eq!(buf.line_ending(), LineEnding::Lf);
}

#[test]
fn from_str_crlf_normalized() {
    let buf = Text::from("hello\r\nworld\r\n");
    // \r stripped — content is pure LF
    assert_eq!(buf.to_string(), "hello\nworld\n");
    assert_eq!(buf.len_chars(), 12); // "hello\nworld\n"
    assert_eq!(buf.line_ending(), LineEnding::CrLf);
}

#[test]
fn from_str_mixed_crlf_lf() {
    // Mixed: CRLF wins if any \r\n present.
    let buf = Text::from("hello\r\nworld\n");
    assert_eq!(buf.to_string(), "hello\nworld\n");
    assert_eq!(buf.line_ending(), LineEnding::CrLf);
}

#[test]
fn from_str_bare_cr_preserved() {
    // Old Mac bare \r is left as-is (treated as content, not a line ending).
    let buf = Text::from("hello\rworld\n");
    assert_eq!(buf.to_string(), "hello\rworld\n");
    assert_eq!(buf.line_ending(), LineEnding::Lf);
}

#[test]
fn from_str_cr_then_crlf_leaves_bare_cr() {
    // "\r\r\n": normalize_crlf is a single forward pass, so the first '\r' is
    // not itself followed by '\n' (its lookahead is the second '\r') and is
    // pushed as-is; only the second '\r' pairs with the following '\n' and is
    // dropped. The rope therefore still contains a literal "\r\n" — this is
    // the case that disproves "content is always \r-free after loading".
    let buf = Text::from("\r\r\n");
    assert_eq!(buf.to_string(), "\r\n");
    assert_eq!(buf.line_ending(), LineEnding::CrLf);
}

#[test]
fn from_str_trailing_newline() {
    // A trailing newline creates an extra empty line.
    let buf = Text::from("hello\n");
    assert_eq!(buf.len_lines(), 2);
}

#[test]
fn line_tokens_count_matches_len_lines_and_keeps_terminators() {
    let buf = Text::from("a\nb\n");
    let tokens: Vec<_> = buf.line_tokens().collect();
    assert_eq!(tokens.len(), buf.len_lines());
    assert_eq!(tokens, vec!["a\n", "b\n", ""]);
}

#[test]
fn line_tokens_splits_on_non_lf_unicode_breaks() {
    // ropey's default `unicode_lines` feature breaks on far more than `\n` —
    // form feed (U+000C) here. `Text::from` only collapses `\r\n`, so a bare
    // FF reaches the rope untouched and still terminates a token.
    let buf = Text::from("a\u{0C}b\n");
    let tokens: Vec<_> = buf.line_tokens().collect();
    assert_eq!(tokens, vec!["a\u{0C}", "b\n", ""]);
}

#[test]
fn line_tokens_at_starts_at_the_requested_line() {
    let buf = Text::from("a\nb\nc\n");
    let tokens: Vec<_> = buf.line_tokens_at(1).collect();
    assert_eq!(tokens, vec!["b\n", "c\n", ""]);
}

#[test]
fn content_line_count_excludes_the_phantom_trailing_line() {
    assert_eq!(Text::from("\n").content_line_count(), 1);
    assert_eq!(Text::from("a\nb\nc\n").content_line_count(), 3);
}

#[test]
fn last_content_line_is_content_line_count_minus_one() {
    assert_eq!(Text::from("\n").last_content_line(), 0);
    assert_eq!(Text::from("a\nb\nc\n").last_content_line(), 2);
}

#[test]
fn from_str_unicode() {
    // "é" can be represented as a single char (U+00E9) or as two chars
    // (U+0065 + U+0301 combining accent). `Text::from` accepts whatever
    // Rust gives us. Here we use the precomposed form — one char.
    let buf = Text::from("café");
    assert_eq!(buf.len_chars(), 5); // c a f é \n
}

#[test]
fn line_to_char() {
    let buf = Text::from("hello\nworld\nfoo");
    assert_eq!(buf.line_to_char(0), 0); // "hello" starts at 0
    assert_eq!(buf.line_to_char(1), 6); // "world" starts after "hello\n"
    assert_eq!(buf.line_to_char(2), 12); // "foo" starts after "world\n"
}

#[test]
fn char_to_line() {
    let buf = Text::from("hello\nworld\nfoo");
    assert_eq!(buf.char_to_line(0), 0); // 'h' is on line 0
    assert_eq!(buf.char_to_line(5), 0); // '\n' is still line 0
    assert_eq!(buf.char_to_line(6), 1); // 'w' is on line 1
    assert_eq!(buf.char_to_line(12), 2); // 'f' is on line 2
}

#[test]
fn insert_at_start() {
    let buf = Text::from("world");
    let new = buf.insert(0, "hello ");
    assert_eq!(new.to_string(), "hello world\n");
    // Original is unchanged — structural sharing.
    assert_eq!(buf.to_string(), "world\n");
}

#[test]
fn insert_at_end() {
    // Insert before the trailing \n (position 5 in "hello\n").
    let buf = Text::from("hello");
    let new = buf.insert(5, " world");
    assert_eq!(new.to_string(), "hello world\n");
}

#[test]
fn insert_in_middle() {
    let buf = Text::from("helo");
    let new = buf.insert(3, "l"); // "hel" + "l" + "o\n"
    assert_eq!(new.to_string(), "hello\n");
}

#[test]
fn remove_whole() {
    let buf = Text::from("hello");
    let new = buf.remove(0..5); // removes "hello", leaving "\n"
    assert_eq!(new.to_string(), "\n");
    assert!(new.is_empty());
    assert_eq!(buf.to_string(), "hello\n"); // original unchanged
}

#[test]
fn remove_range() {
    let buf = Text::from("hello world");
    let new = buf.remove(5..11); // remove " world"
    assert_eq!(new.to_string(), "hello\n");
}

#[test]
fn insert_then_remove_is_identity() {
    let original = Text::from("hello world");
    let after_insert = original.insert(5, " beautiful");
    let restored = after_insert.remove(5..15);
    assert_eq!(restored, original);
}

#[test]
fn slice() {
    let buf = Text::from("hello world");
    let s: String = buf.slice(6..11).to_string();
    assert_eq!(s, "world");
}

#[test]
fn equality() {
    let a = Text::from("hello");
    let b = Text::from("hello");
    let c = Text::from("world");
    assert_eq!(a, b);
    assert_ne!(a, c);
}

// ── char_at boundary cases ────────────────────────────────────────────────

#[test]
fn char_at_first_position() {
    let buf = Text::from("hello");
    assert_eq!(buf.char_at(0), Some('h'));
}

#[test]
fn char_at_last_position() {
    // "hello" + structural '\n' → last char is '\n' at len_chars()-1.
    let buf = Text::from("hello");
    assert_eq!(buf.char_at(buf.len_chars() - 1), Some('\n'));
}

#[test]
fn char_at_out_of_bounds() {
    let buf = Text::from("hello");
    assert_eq!(buf.char_at(buf.len_chars()), None);
}

// ── single-char buffer ────────────────────────────────────────────────────

#[test]
fn single_char_buffer_has_two_chars() {
    // "x" gets the structural '\n' appended → len_chars() == 2.
    let buf = Text::from("x");
    assert_eq!(buf.len_chars(), 2);
    assert!(!buf.is_empty());
    assert_eq!(buf.char_at(0), Some('x'));
    assert_eq!(buf.char_at(1), Some('\n'));
}

// ── remove with empty range ───────────────────────────────────────────────

#[test]
fn remove_empty_range_is_identity() {
    let buf = Text::from("hello");
    let same = buf.remove(3..3);
    assert_eq!(same.to_string(), "hello\n");
}

// ── insert/remove with multi-byte content ─────────────────────────────────

#[test]
fn insert_grapheme_cluster() {
    // Insert a two-char grapheme (e + combining acute) at position 0.
    let buf = Text::from("hello");
    let new = buf.insert(0, "e\u{0301}");
    // 'e' + U+0301 + "hello" + '\n' = 8 chars.
    assert_eq!(new.len_chars(), 8);
    assert_eq!(new.char_at(0), Some('e'));
    assert_eq!(new.char_at(1), Some('\u{0301}'));
    assert_eq!(new.char_at(2), Some('h'));
}

#[test]
fn remove_grapheme_cluster_range() {
    // Remove a two-char grapheme cluster.
    let buf = Text::from("e\u{0301}hello");
    // buf: 'e'(0) U+0301(1) 'h'(2) 'e'(3) ... '\n'(7) = 8 chars.
    let new = buf.remove(0..2); // remove the 'e' + combining accent
    assert_eq!(new.to_string(), "hello\n");
}

// ── CharCursor ────────────────────────────────────────────────────────────

#[test]
fn char_cursor_forward_from_start() {
    // "hello\n": h0 e1 l2 l3 o4 \n5.
    let buf = Text::from("hello");
    let got: Vec<(usize, char)> = buf.chars_at(0).collect();
    assert_eq!(
        got,
        vec![(0, 'h'), (1, 'e'), (2, 'l'), (3, 'l'), (4, 'o'), (5, '\n')]
    );
}

#[test]
fn char_cursor_forward_from_middle() {
    let buf = Text::from("hello");
    let got: Vec<(usize, char)> = buf.chars_at(2).collect();
    assert_eq!(got, vec![(2, 'l'), (3, 'l'), (4, 'o'), (5, '\n')]);
}

#[test]
fn char_cursor_prev_walks_back_to_start_then_none() {
    let buf = Text::from("hello");
    let mut c = buf.chars_at(4); // positioned before 'o'
    assert_eq!(c.prev(), Some((3, 'l')));
    assert_eq!(c.prev(), Some((2, 'l')));
    assert_eq!(c.prev(), Some((1, 'e')));
    assert_eq!(c.prev(), Some((0, 'h')));
    assert_eq!(c.prev(), None);
}

#[test]
fn char_cursor_interleaved_next_prev_round_trips() {
    let buf = Text::from("hello");
    let mut c = buf.chars_at(2);
    assert_eq!(c.next(), Some((2, 'l'))); // cursor now at 3
    assert_eq!(c.prev(), Some((2, 'l'))); // back to 2 — same value
    assert_eq!(c.next(), Some((2, 'l'))); // forward again — still consistent
}

#[test]
fn char_cursor_at_eof() {
    let buf = Text::from("hello");
    let len = buf.len_chars();
    let mut at_eof = buf.chars_at(len);
    assert_eq!(at_eof.next(), None);
    assert_eq!(at_eof.prev(), Some((len - 1, '\n')));
}

#[test]
fn char_cursor_yields_codepoints_not_grapheme_clusters() {
    // "caf" + e + U+0301 (combining acute, 2 codepoints) + structural \n:
    // c0 a1 f2 e3 U+0301(4) \n(5). The combining mark must come back as
    // its own char, not merged with 'e' — CharCursor is char-level.
    let buf = Text::from("caf\u{0065}\u{0301}");
    let got: Vec<(usize, char)> = buf.chars_at(3).collect();
    assert_eq!(got, vec![(3, 'e'), (4, '\u{0301}'), (5, '\n')]);
}

#[test]
fn strip_line_break_strips_every_unicode_line_break() {
    // Independent oracle: each expected value is a literal, not derived
    // from LINE_BREAKS — a bug that drops one break char from the const
    // still fails this.
    assert_eq!(strip_line_break("hello\n"), "hello");
    assert_eq!(strip_line_break("hello\r"), "hello");
    assert_eq!(strip_line_break("hello\u{0B}"), "hello"); // VT
    assert_eq!(strip_line_break("hello\u{0C}"), "hello"); // FF
    assert_eq!(strip_line_break("hello\u{85}"), "hello"); // NEL
    assert_eq!(strip_line_break("hello\u{2028}"), "hello"); // LS
    assert_eq!(strip_line_break("hello\u{2029}"), "hello"); // PS
}

#[test]
fn strip_line_break_collapses_crlf_in_one_pass() {
    assert_eq!(strip_line_break("hello\r\n"), "hello");
}

#[test]
fn strip_line_break_is_a_no_op_without_a_trailing_break() {
    assert_eq!(strip_line_break("hello"), "hello");
}
