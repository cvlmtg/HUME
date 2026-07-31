use super::super::*;
use hume_test_fixtures::assert_state;
use pretty_assertions::assert_eq;

// ── replace_around_cursors ───────────────────────────────────────────────────
//
// The multi-cursor "as if typed" primitive behind LSP completion accept:
// replaces `back` chars behind each cursor's head and `forward` chars ahead
// of it with `text`, uniformly at every selection.

#[test]
fn replace_around_cursors_single_cursor_baseline() {
    // head=2 ('l'); back=2 deletes "he"; forward=0 leaves the head char and
    // everything after untouched.
    assert_state!(
        "he-[l]>lo\n",
        |(buf, sels)| replace_around_cursors(buf, sels, 2, 0, "XYZ"),
        "XYZ-[l]>lo\n"
    );
}

#[test]
fn replace_around_cursors_two_cursors_uniform_spacing() {
    // The op-level shape of the multi-cursor completion bug: two cursors,
    // each right after its own typed "st", both get the same replacement.
    assert_state!(
        "st-[ ]>st-[\n]>",
        |(buf, sels)| replace_around_cursors(buf, sels, 2, 0, "XY"),
        "XY-[ ]>XY-[\n]>"
    );
}

#[test]
fn replace_around_cursors_forward_consumes_chars_ahead_of_head() {
    // head=2 ('l'); back=1 deletes "e"; forward=1 also consumes the head
    // char itself ('l') — completing in the middle of a token, where the
    // server's range extends past the live cursor.
    assert_state!(
        "he-[l]>lo\n",
        |(buf, sels)| replace_around_cursors(buf, sels, 1, 1, "XYZ"),
        "hXYZ-[l]>o\n"
    );
}

#[test]
fn replace_around_cursors_clamps_underflow_at_buffer_start() {
    // head=2 ('c'); back=5 asks for more chars than exist before the cursor
    // — clamped to the buffer start (0) rather than underflowing.
    assert_state!(
        "ab-[c]>de\n",
        |(buf, sels)| replace_around_cursors(buf, sels, 5, 0, "Z"),
        "Z-[c]>de\n"
    );
}

#[test]
fn replace_around_cursors_clamps_when_cursors_are_closer_than_back() {
    // Cursors at 'c' and 'd' (1 char apart) with back=2 each: the ideal
    // start for the second cursor (1 char before 'c') would fall inside
    // territory the first cursor's edit already claimed. Clamped to the
    // first edit's end instead of erroring — the second cursor still gets
    // "Z", it just eats one char ('c') instead of two ("b","c").
    assert_state!(
        "ab-[c]>-[d]>ef\n",
        |(buf, sels)| replace_around_cursors(buf, sels, 2, 0, "Z"),
        "Z-[Z]>-[d]>ef\n"
    );
}

#[test]
fn replace_around_cursors_snaps_start_outward_past_a_combining_mark() {
    // "café" = c,a,f,e,{combining acute} — one grapheme cluster spans chars
    // [3,5). head=6 ('x'), back=2 puts the raw start at char 4, splitting
    // the cluster between the base 'e' and its accent. The snap floors it
    // to 3, deleting the whole cluster instead of orphaning the accent.
    assert_state!(
        "cafe\u{0301} -[x]>\n",
        |(buf, sels)| replace_around_cursors(buf, sels, 2, 0, "Z"),
        "cafZ-[x]>\n"
    );
}

#[test]
fn replace_around_cursors_zero_span_matches_insert_str() {
    // back=0 forward=0 degenerates to a pure multi-cursor insert.
    // Independent oracle: insert_str is a separately implemented op, so
    // agreement here isn't circular against replace_around_cursors's own
    // logic.
    let buf = Text::from("foo bar\n");
    let sels = SelectionSet::from_vec(vec![Selection::collapsed(0), Selection::collapsed(4)], 0);
    let (buf_replace, sels_replace, cs_replace) =
        replace_around_cursors(buf.clone(), sels.clone(), 0, 0, "X");
    let (buf_insert, sels_insert, cs_insert) = insert_str(buf, sels, "X");
    assert_eq!(buf_replace.to_string(), buf_insert.to_string());
    assert_eq!(sels_replace, sels_insert);
    assert_eq!(cs_replace, cs_insert);
}

#[test]
fn replace_around_cursors_does_not_delete_the_structural_trailing_newline_after_a_bare_cr() {
    // A source ending in a lone `\r` (old-Mac line ending) is left as-is by
    // `normalize_crlf` (only `\r\n` pairs are stripped), then `Text::from`
    // appends the buffer's own structural trailing `\n` — so the rope ends
    // in the two-char cluster `\r\n`. `forward` reaching past the end must
    // floor back to that cluster's start instead of ceiling through it and
    // deleting the structural newline.
    let buf = Text::from("ab\r");
    assert_eq!(
        buf.to_string(),
        "ab\r\n",
        "sanity: bare CR survives, \\n is appended"
    );
    let sels = SelectionSet::from_vec(vec![Selection::collapsed(0)], 0);
    let (new_buf, new_sels, _cs) = replace_around_cursors(buf, sels, 0, 10, "X");
    assert_eq!(
        new_buf.to_string(),
        "X\r\n",
        "structural trailing newline (and the CR before it) must survive"
    );
    assert!(
        new_sels.primary().head() < new_buf.len_chars(),
        "cursor must land before the structural newline, not on/after it"
    );
}

// ── replace_span_around_cursors (per-cursor `start_of`) ──────────────────────
//
// `replace_around_cursors` (tested above) is the uniform-`back` case; these
// exercise `replace_span_around_cursors` directly with a `word_start_before`-
// based `start_of`, the shape LSP completion's `insertText` fallback uses for
// every cursor but its own primary (see `completion.rs`'s `accept`).

#[test]
fn replace_span_around_cursors_word_start_before_uses_each_cursors_own_token_length() {
    // Primary's token is "foo" (3 chars); the second cursor's own preceding
    // token is "o" (1 char, preceded by punctuation) — using the primary's
    // token length there (as a naive uniform `back` would) eats "x(" too.
    // Independent oracle: this fails under `replace_around_cursors` with a
    // hardcoded `back = 3`, so it isn't circular against the fix.
    assert_state!(
        "foo-[ ]>x(o-[)]>\n",
        |(buf, sels)| replace_span_around_cursors(buf, sels, word_start_before, 0, "Z"),
        "Z-[ ]>x(Z-[)]>\n"
    );
}

#[test]
fn replace_span_around_cursors_skips_typed_chars_before_scanning_each_cursors_prefix() {
    // Both cursors typed the same 2 chars ("go") since the session began —
    // uniform, real multi-cursor typing keeps every cursor in lockstep — but
    // each had a different pre-existing token before that: "foo" ahead of
    // the first, "o" (preceded by punctuation) ahead of the second. The
    // typed suffix must be skipped before each cursor's own backward scan
    // starts, or the scan would walk into the "go" every cursor typed
    // instead of stopping at each cursor's own pre-session content.
    let typed = 2;
    assert_state!(
        "foogo-[ ]>x(ogo-[)]>\n",
        |(buf, sels)| replace_span_around_cursors(
            buf,
            sels,
            move |buf, head| word_start_before(buf, head.saturating_sub(typed)),
            0,
            "Z"
        ),
        "Z-[ ]>x(Z-[)]>\n"
    );
}

// ── replace_selections ────────────────────────────────────────────────────

#[test]
fn replace_cursor_single_char() {
    // Cursor on 'h'; replace with 'x' → cursor stays on 'x'.
    assert_state!(
        "-[h]>ello\n",
        |(buf, sels)| replace_selections(buf, sels, 'x'),
        "-[x]>ello\n"
    );
}

#[test]
fn replace_cursor_middle() {
    // Cursor on 'l' at offset 2; replace with 'x'.
    assert_state!(
        "he-[l]>lo\n",
        |(buf, sels)| replace_selections(buf, sels, 'x'),
        "he-[x]>lo\n"
    );
}

#[test]
fn replace_cursor_on_structural_newline_is_noop() {
    // Structural trailing '\n' is skipped like any other '\n'.
    assert_state!(
        "hello-[\n]>",
        |(buf, sels)| replace_selections(buf, sels, 'x'),
        "hello-[\n]>"
    );
}

#[test]
fn replace_cursor_on_mid_buffer_newline_is_noop() {
    // Cursor on the '\n' between two lines — preserved, not replaced.
    assert_state!(
        "hello-[\n]>world\n",
        |(buf, sels)| replace_selections(buf, sels, 'x'),
        "hello-[\n]>world\n"
    );
}

#[test]
fn replace_empty_buffer_is_noop() {
    // Text is just the structural '\n'.
    assert_state!(
        "-[\n]>",
        |(buf, sels)| replace_selections(buf, sels, 'x'),
        "-[\n]>"
    );
}

#[test]
fn replace_forward_selection() {
    // Forward selection covers "hell" (offsets 0-3); replace each with 'x'.
    assert_state!(
        "-[hell]>o\n",
        |(buf, sels)| replace_selections(buf, sels, 'x'),
        "-[xxxx]>o\n"
    );
}

#[test]
fn replace_backward_selection() {
    // Backward selection anchor=3, head=0 covers "hell"; direction preserved.
    assert_state!(
        "<[hell]-o\n",
        |(buf, sels)| replace_selections(buf, sels, 'x'),
        "<[xxxx]-o\n"
    );
}

#[test]
fn replace_whole_line() {
    // Forward selection covers all content chars (not the structural '\n').
    assert_state!(
        "-[hello]>\n",
        |(buf, sels)| replace_selections(buf, sels, 'x'),
        "-[xxxxx]>\n"
    );
}

#[test]
fn replace_two_cursors() {
    // Two cursors; each independently replaced.
    assert_state!(
        "-[h]>ell-[o]>\n",
        |(buf, sels)| replace_selections(buf, sels, 'x'),
        "-[x]>ell-[x]>\n"
    );
}

#[test]
fn replace_two_selections() {
    // Two non-overlapping selections each get all their chars replaced.
    assert_state!(
        "-[he]>l-[lo]>\n",
        |(buf, sels)| replace_selections(buf, sels, 'x'),
        "-[xx]>l-[xx]>\n"
    );
}

#[test]
fn replace_grapheme_cluster_cursor() {
    // Cursor on 'é' (e + U+0301, 2 codepoints). Replaced with 'x' (1 codepoint).
    // Text shrinks by 1 char; cursor lands on 'x'.
    assert_state!(
        "caf-[e]>\u{0301}z\n",
        |(buf, sels)| replace_selections(buf, sels, 'x'),
        "caf-[x]>z\n"
    );
}

#[test]
fn replace_multiline_selection_skips_newline() {
    // Selection spans two lines. The '\n' between them is retained;
    // only the visible characters are replaced. Lines stay separate.
    assert_state!(
        "-[hello\nworld]>\n",
        |(buf, sels)| replace_selections(buf, sels, 'x'),
        "-[xxxxx\nxxxxx]>\n"
    );
}

#[test]
fn replace_selection_including_structural_trailing_newline_preserves_newline() {
    // When the selection reaches the structural trailing '\n', that newline
    // must be preserved — replace_selections skips '\n' graphemes entirely.
    // Before the fix this path existed but had no explicit test.
    assert_state!(
        "-[hello\n]>",
        |(buf, sels)| replace_selections(buf, sels, 'x'),
        "-[xxxxx\n]>"
    );
}

// ── Smart replace (pair-aware) ───────────────────────────────────────────

#[test]
fn smart_replace_opening_bracket_to_opening() {
    // Two cursors on `(` and `)`, replace with `[` → `[` and `]`.
    assert_state!(
        "-[(]>hello-[)]>\n",
        |(buf, sels)| replace_selections(buf, sels, '['),
        "-[[]>hello-[]]>\n"
    );
}

#[test]
fn smart_replace_asym_to_sym() {
    // `(` and `)` replaced with `"` → both become `"`.
    assert_state!(
        "-[(]>hello-[)]>\n",
        |(buf, sels)| replace_selections(buf, sels, '"'),
        "-[\"]>hello-[\"]>\n"
    );
}

#[test]
fn smart_replace_sym_to_asym_uses_index() {
    // Two cursors on `"` and `"`, replace with `(` → `(` and `)`.
    assert_state!(
        "-[\"]>hello-[\"]>\n",
        |(buf, sels)| replace_selections(buf, sels, '('),
        "-[(]>hello-[)]>\n"
    );
}

#[test]
fn smart_replace_sym_to_sym() {
    // Two cursors on `"` and `"`, replace with `'` → both `'`.
    assert_state!(
        "-[\"]>hello-[\"]>\n",
        |(buf, sels)| replace_selections(buf, sels, '\''),
        "-[']>hello-[']>\n"
    );
}

#[test]
fn smart_replace_non_delimiter_is_literal() {
    // Cursor on `x`, replace with `[` → literal `[` (no smart logic).
    assert_state!(
        "-[x]>hello\n",
        |(buf, sels)| replace_selections(buf, sels, '['),
        "-[[]>hello\n"
    );
}

#[test]
fn smart_replace_range_selection_no_smart_logic() {
    // Range selection (not a cursor) — all chars become `[`, no smart logic.
    assert_state!(
        "-[(he]>llo)\n",
        |(buf, sels)| replace_selections(buf, sels, '['),
        "-[[[[]>llo)\n"
    );
}

#[test]
fn smart_replace_non_pair_replacement_is_literal() {
    // Replacement is not a pair char — always literal, even on delimiters.
    assert_state!(
        "-[(]>hello-[)]>\n",
        |(buf, sels)| replace_selections(buf, sels, 'x'),
        "-[x]>hello-[x]>\n"
    );
}
