/// Test DSL for HUME editing operations.
///
/// We use a compact, human-readable string format to express editor state
/// (buffer content + selections) inline in test source code.
///
/// # Marker format
///
/// | Marker | Meaning |
/// |--------|---------|
/// | `-[`   | Anchor side of a selection bracket. |
/// | `]>`   | Head (cursor) side — forward direction. |
/// | `<[`   | Head (cursor) side — backward direction. |
/// | `]-`   | Anchor side closing a backward selection. |
///
/// ## Selection syntax
///
/// ```text
/// -[hell]>o\n      — forward selection:  anchor=0, head=3 (cursor on 'l', selects "hell")
/// <[hell]-o\n      — backward selection: head=0, anchor=3 (cursor on 'h', selects "hell")
/// hel-[l]>o\n      — cursor on 'l' (anchor == head == 3, same as 1-char forward selection)
/// ```
///
/// ## Key properties
///
/// - The text between `[` and `]` is **exactly** the selected text (inclusive of both
///   anchor and head characters).
/// - `-` always marks the **anchor** end; `>` / `<` always marks the **head** (cursor) end.
///   The arrow direction shows which way the selection faces.
/// - A cursor (anchor == head) is just a 1-char forward selection: `-[x]>`.
/// - Multiple selections in one string: `-[he]>llo -[wor]>ld\n`
///
/// ## Cursor model
///
/// The cursor is *inclusive* — it sits **on** a character, not between characters.
/// `head` is the char offset of the cursor character.
///
/// For `-[hell]>o world\n`: anchor=0, head=3 (cursor is on the second 'l').
/// For `<[hell]-o world\n`: head=0, anchor=3 (cursor is on 'h').
use crate::buffer::Buffer;
use crate::selection::{Selection, SelectionSet};

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Count the number of Unicode scalar values in `s`.
///
/// We use `str::chars().count()` which is O(n) in the byte length of the
/// string. Since this is called only during test setup (not in hot paths)
/// that is perfectly acceptable.
#[inline]
fn char_count(s: &str) -> usize {
    s.chars().count()
}

// ── State parsing ─────────────────────────────────────────────────────────────

/// Parse a marker-annotated string into `(Buffer, SelectionSet)`.
///
/// The markers are stripped from the returned buffer. Panics with a
/// descriptive message if the string contains no selection markers, or if a
/// marker is malformed (e.g. a `-[` with no matching `]>`).
pub(crate) fn parse_state(input: &str) -> (Buffer, SelectionSet) {
    // We scan the input one char at a time, building up:
    //   - `text`:       the raw buffer content (markers removed)
    //   - `selections`: the parsed Selection values
    //
    // State machine:
    //   Normal           — outside any selection marker
    //   InForward(pos)   — inside -[…]>, anchor recorded at `pos`
    //   InBackward(pos)  — inside <[…]-, head recorded at `pos`
    //
    // Two-char tokens (recognised by peeking one char ahead):
    //   `-[`  — open forward selection (anchor at current char_count)
    //   `]>`  — close forward selection (head = char_count - 1)
    //   `<[`  — open backward selection (head at current char_count)
    //   `]-`  — close backward selection (anchor = char_count - 1)
    //
    // Any char that does not start a two-char token is appended to `text`.

    let mut text = String::with_capacity(input.len());
    let mut selections: Vec<Selection> = Vec::new();

    #[derive(Debug)]
    enum State {
        Normal,
        /// Inside `-[…]>`: anchor was recorded at `anchor_offset`.
        InForward { anchor_offset: usize },
        /// Inside `<[…]-`: head was recorded at `head_offset`.
        InBackward { head_offset: usize },
    }

    let mut state = State::Normal;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match (&state, ch) {
            // ── Open forward: `-[` ────────────────────────────────────────
            (State::Normal, '-') if chars.peek() == Some(&'[') => {
                chars.next(); // consume '['
                state = State::InForward { anchor_offset: char_count(&text) };
            }

            // ── Open backward: `<[` ───────────────────────────────────────
            (State::Normal, '<') if chars.peek() == Some(&'[') => {
                chars.next(); // consume '['
                state = State::InBackward { head_offset: char_count(&text) };
            }

            // ── Close forward: `]>` ───────────────────────────────────────
            (State::InForward { anchor_offset }, ']') if chars.peek() == Some(&'>') => {
                chars.next(); // consume '>'
                let count = char_count(&text);
                assert!(
                    count > *anchor_offset,
                    "parse_state: empty selection `-[]>` in {:?} — \
                     a selection must cover at least one character",
                    input
                );
                let head = count - 1; // last char written is the head
                selections.push(Selection::new(*anchor_offset, head));
                state = State::Normal;
            }

            // ── Close backward: `]-` ──────────────────────────────────────
            (State::InBackward { head_offset }, ']') if chars.peek() == Some(&'-') => {
                chars.next(); // consume '-'
                let count = char_count(&text);
                assert!(
                    count > *head_offset,
                    "parse_state: empty selection `<[]-` in {:?} — \
                     a selection must cover at least one character",
                    input
                );
                let anchor = count - 1; // last char written is the anchor
                selections.push(Selection::new(anchor, *head_offset));
                state = State::Normal;
            }

            // ── Guard: `]` not followed by `>` or `-` is literal text ─────
            (_, ']') => {
                text.push(']');
            }

            // ── Guard: lone `-` not followed by `[` is literal text ───────
            (_, '-') => {
                text.push('-');
            }

            // ── Guard: lone `<` not followed by `[` is literal text ───────
            (_, '<') => {
                text.push('<');
            }

            // ── Regular character — append to buffer text ─────────────────
            (_, c) => {
                text.push(c);
            }
        }
    }

    // Validate that the markers were properly closed.
    match state {
        State::InForward { .. } => panic!(
            "parse_state: unterminated `-[` in input: {:?}\n\
             Did you forget the closing `]>`?",
            input
        ),
        State::InBackward { .. } => panic!(
            "parse_state: unterminated `<[` in input: {:?}\n\
             Did you forget the closing `]-`?",
            input
        ),
        State::Normal => {}
    }

    assert!(
        !selections.is_empty(),
        "parse_state: no selection markers found in input: {:?}\n\
         Add at least one `-[x]>` cursor or `-[text]>` / `<[text]-` selection.",
        input
    );

    assert!(
        text.ends_with('\n'),
        "parse_state: DSL string must produce a buffer ending with '\\n' (got {:?}).\n\
         Every buffer has a structural trailing newline — include it explicitly.\n\
         E.g. use \"-[h]>ello\\n\" not \"-[h]>ello\", \"hello-[\\n]>\" not \"hello-[]\", \
         \"-[\\n]>\" not \"-[]>\".",
        input
    );

    let buf = Buffer::from_str(&text);
    let sel_set = SelectionSet::from_vec(selections, 0);
    (buf, sel_set)
}

/// Serialize `(Buffer, SelectionSet)` back to the marker format.
///
/// This is the inverse of `parse_state`. It is used in assertions so that
/// diffs show the annotated marker text rather than raw char offsets.
pub(crate) fn serialize_state(buf: &Buffer, sels: &SelectionSet) -> String {
    let full = buf.to_string();
    // Include the structural trailing \n in the serialized output so that
    // DSL strings are explicit about buffer content. Every valid buffer ends
    // with \n, so every serialized string ends with \n too.
    let text = &full;
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();

    // Build a lookup: char_offset → what markers to insert before this char.
    // We use a `Vec` of vecs indexed by char position, plus a special slot
    // at index `n` for markers that appear after the last character.
    //
    // Selections are processed in sorted order (iter_sorted), so closing markers
    // of one selection are naturally added before opening markers of the next
    // when they share the same position — producing `]>-[` not `-[]>` etc.
    let mut markers: Vec<Vec<&'static str>> = vec![vec![]; n + 1];

    for sel in sels.iter_sorted() {
        if sel.anchor <= sel.head {
            // Forward selection (including cursor where anchor == head).
            // `-[` at anchor, `]>` one past head.
            markers[sel.anchor].push("-[");
            markers[(sel.head + 1).min(n)].push("]>");
        } else {
            // Backward selection (anchor > head).
            // `<[` at head, `]-` one past anchor.
            markers[sel.head].push("<[");
            markers[(sel.anchor + 1).min(n)].push("]-");
        }
    }

    let mut out = String::with_capacity(text.len() + sels.len() * 8);
    for i in 0..=n {
        for &marker in &markers[i] {
            out.push_str(marker);
        }
        if i < n {
            out.push(chars[i]);
        }
    }
    out
}

// ── Assertion macro ───────────────────────────────────────────────────────────

/// Assert that applying `$op` to the state described by `$initial` produces
/// the state described by `$expected`.
///
/// Both `$initial` and `$expected` are marker-annotated strings (see module
/// docs for the format). `$op` is a closure that takes `(Buffer, SelectionSet)`
/// and returns `(Buffer, SelectionSet)`.
///
/// # Example
///
/// ```
/// assert_state!(
///     "-[h]>ello\n",                                // initial state
///     |(buf, sels)| delete_char_forward(buf, sels), // operation
///     "-[e]>llo\n",                                 // expected state
/// );
/// ```
///
/// On failure the error message shows both sides in marker format, making it
/// immediately obvious what went wrong.
#[macro_export]
macro_rules! assert_state {
    ($initial:expr, $op:expr, $expected:expr) => {{
        use $crate::testing::{parse_state, serialize_state};
        use pretty_assertions::assert_eq;

        let (buf, sels) = parse_state($initial);
        let (result_buf, result_sels) = $op((buf, sels));
        let (expected_buf, expected_sels) = parse_state($expected);

        assert_eq!(
            serialize_state(&result_buf, &result_sels),
            serialize_state(&expected_buf, &expected_sels),
        );
    }};
}

// ── Tests for the DSL itself ──────────────────────────────────────────────────
//
// A test that depends on a broken test helper is worse than no test at all.
// We thoroughly test `parse_state` and `serialize_state` before using them
// in any editing operation tests.

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    // ── parse_state ───────────────────────────────────────────────────────────

    #[test]
    fn parse_cursor_at_start() {
        // -[h]>ello\n — cursor on 'h' (offset 0), anchor == head == 0
        let (buf, sels) = parse_state("-[h]>ello\n");
        assert_eq!(buf.to_string(), "hello\n");
        assert_eq!(sels.len(), 1);
        let s = sels.primary();
        assert!(s.is_cursor());
        assert_eq!(s.head, 0);
        assert_eq!(s.anchor, 0);
    }

    #[test]
    fn parse_cursor_at_end() {
        // hello-[\n]> — cursor on '\n' (offset 5)
        let (buf, sels) = parse_state("hello-[\n]>");
        assert_eq!(buf.to_string(), "hello\n");
        assert_eq!(sels.primary().head, 5);
    }

    #[test]
    fn parse_cursor_in_middle() {
        // hel-[l]>o\n — cursor on second 'l' (offset 3)
        let (buf, sels) = parse_state("hel-[l]>o\n");
        assert_eq!(buf.to_string(), "hello\n");
        assert_eq!(sels.primary().head, 3);
    }

    #[test]
    fn parse_forward_selection() {
        // -[hell]>o world\n — anchor=0, head=3 (selects "hell")
        let (buf, sels) = parse_state("-[hell]>o world\n");
        assert_eq!(buf.to_string(), "hello world\n");
        let s = sels.primary();
        assert_eq!(s.anchor, 0);
        assert_eq!(s.head, 3);
    }

    #[test]
    fn parse_backward_selection() {
        // <[hel]-lo\n — head=0, anchor=2 (cursor on 'h', selects "hel")
        let (buf, sels) = parse_state("<[hel]-lo\n");
        assert_eq!(buf.to_string(), "hello\n");
        let s = sels.primary();
        assert_eq!(s.anchor, 2);
        assert_eq!(s.head, 0);
    }

    #[test]
    fn parse_selection_near_end_of_buffer() {
        // hi -[ther]>e\n — anchor=3, head=6 (selects "ther")
        let (buf, sels) = parse_state("hi -[ther]>e\n");
        assert_eq!(buf.to_string(), "hi there\n");
        let s = sels.primary();
        assert_eq!(s.anchor, 3);
        assert_eq!(s.head, 6);
    }

    #[test]
    fn parse_two_cursors() {
        // -[f]>oo-[ ]>bar\n — cursors on 'f' (0) and ' ' (3)
        let (buf, sels) = parse_state("-[f]>oo-[ ]>bar\n");
        assert_eq!(buf.to_string(), "foo bar\n");
        assert_eq!(sels.len(), 2);
        assert_eq!(sels.iter_sorted().next().unwrap().head, 0);
        assert_eq!(sels.iter_sorted().nth(1).unwrap().head, 3);
    }

    #[test]
    fn parse_two_forward_selections() {
        // -[ab]> -[de]>\n — (anchor=0,head=1) and (anchor=4,head=5)
        let (buf, sels) = parse_state("-[ab]> -[de]>\n");
        assert_eq!(buf.to_string(), "ab de\n");
        assert_eq!(sels.len(), 2);
        let mut it = sels.iter_sorted();
        let s0 = it.next().unwrap();
        let s1 = it.next().unwrap();
        assert_eq!((s0.anchor, s0.head), (0, 1));
        assert_eq!((s1.anchor, s1.head), (3, 4));
    }

    #[test]
    fn parse_cursor_on_unicode_char() {
        // "é" is U+00E9, a single Unicode scalar value (1 char).
        let (buf, sels) = parse_state("caf-[é]>\n");
        assert_eq!(buf.to_string(), "café\n");
        assert_eq!(sels.primary().head, 3);
    }

    #[test]
    fn parse_cursor_on_only_newline() {
        // -[\n]> — cursor on '\n' in a buffer that contains only the trailing newline
        let (buf, sels) = parse_state("-[\n]>");
        assert_eq!(buf.to_string(), "\n");
        assert_eq!(sels.primary().head, 0);
    }

    #[test]
    fn parse_literal_dash_and_angle_in_buffer() {
        // Lone `-` and `<` (not followed by `[`) are plain buffer content.
        let (buf, sels) = parse_state("-[x]>a-b<c\n");
        assert_eq!(buf.to_string(), "xa-b<c\n");
        assert_eq!(sels.primary().head, 0);
    }

    // ── serialize_state ───────────────────────────────────────────────────────

    #[test]
    fn serialize_cursor_at_start() {
        let buf = Buffer::from_str("hello");
        let sels = SelectionSet::single(Selection::cursor(0));
        assert_eq!(serialize_state(&buf, &sels), "-[h]>ello\n");
    }

    #[test]
    fn serialize_cursor_at_end() {
        // cursor at 5 = on the structural trailing \n.
        let buf = Buffer::from_str("hello");
        let sels = SelectionSet::single(Selection::cursor(5));
        assert_eq!(serialize_state(&buf, &sels), "hello-[\n]>");
    }

    #[test]
    fn serialize_forward_selection() {
        // anchor=0, head=3 — selects "hell" (positions 0..=3).
        let buf = Buffer::from_str("hello world");
        let sels = SelectionSet::single(Selection::new(0, 3));
        assert_eq!(serialize_state(&buf, &sels), "-[hell]>o world\n");
    }

    #[test]
    fn serialize_backward_selection() {
        // anchor=3, head=0 — selects "hell" (positions 0..=3), cursor on 'h'.
        let buf = Buffer::from_str("hello");
        let sels = SelectionSet::single(Selection::new(3, 0));
        assert_eq!(serialize_state(&buf, &sels), "<[hell]-o\n");
    }

    #[test]
    fn serialize_forward_selection_head_at_eof() {
        // head=5 is the trailing \n in "hello\n". Selects "hello\n".
        let buf = Buffer::from_str("hello");
        let sels = SelectionSet::single(Selection::new(0, 5));
        assert_eq!(serialize_state(&buf, &sels), "-[hello\n]>");
    }

    // ── Round-trip ────────────────────────────────────────────────────────────

    fn round_trip(s: &str) -> String {
        let (buf, sels) = parse_state(s);
        serialize_state(&buf, &sels)
    }

    #[test]
    fn roundtrip_cursor() {
        assert_eq!(round_trip("-[h]>ello\n"), "-[h]>ello\n");
        assert_eq!(round_trip("hello-[\n]>"), "hello-[\n]>");
        assert_eq!(round_trip("hel-[l]>o\n"), "hel-[l]>o\n");
    }

    #[test]
    fn roundtrip_forward_selection() {
        assert_eq!(round_trip("-[hell]>o world\n"), "-[hell]>o world\n");
    }

    #[test]
    fn roundtrip_backward_selection() {
        assert_eq!(round_trip("<[hel]-lo\n"), "<[hel]-lo\n");
    }

    #[test]
    fn roundtrip_two_cursors() {
        assert_eq!(round_trip("-[f]>oo-[ ]>bar\n"), "-[f]>oo-[ ]>bar\n");
    }

    #[test]
    fn roundtrip_newline_only_buffer() {
        assert_eq!(round_trip("-[\n]>"), "-[\n]>");
    }
}
