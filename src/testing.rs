/// Test DSL for HUME editing operations.
///
/// We use a compact, human-readable string format to express editor state
/// (buffer content + selections) inline in test source code. This is the same
/// format Helix uses for its own tests, so Helix test cases can be ported
/// directly.
///
/// # Marker format
///
/// | Marker | Meaning |
/// |--------|---------|
/// | `\|`   | Collapsed cursor (anchor == head). |
/// | `#[`   | Start of a selection (anchor position). |
/// | `\|` inside `#[…]#` | Head (cursor) position. **Forward** `#[ABC\|C]#`: ABC are the selected chars before the cursor; C is the single cursor character (at `head`). **Backward** `#[\|CCC]#`: `\|` is right after `#[` and CCC are the selected chars after the cursor. |
/// | `]#`   | End of a selection. For forward selections, placed one past `head` (i.e. after the cursor char). For backward selections, placed at `anchor`. |
///
/// ## Examples
///
/// ```text
/// "|hello"           — cursor on 'h' (offset 0)
/// "hello|"           — cursor at end of buffer (offset 5, past last char)
/// "hel|lo"           — cursor on the second 'l' (offset 3)
/// "#[hel|l]#o world" — forward selection: anchor=0, head=3 (cursor on 'l')
/// "#[|hel]#lo"       — backward selection: anchor=3, head=0 (cursor on 'h')
/// "#[a|b]# #[c|d]#"  — two forward selections
/// ```
///
/// ## Cursor model
///
/// Helix's cursor is *inclusive* — it sits **on** a character, not between
/// characters. `head` is the char offset of the cursor character.
///
/// In a forward selection `#[ABC|C]#`:
/// - `ABC` is the text from `anchor` up to (but not including) `head`.
/// - `C` (a single char) is the character **at** `head` — the cursor character.
/// - `]#` goes at `head + 1` (one past the cursor char).
/// - `anchor` = position of `#[`, `head` = position of `|`.
///
/// So `#[hel|l]#o world` means: anchor=0, head=3 (cursor is on the second
/// 'l' at index 3; 'o' and the rest are unselected).
use crate::buffer::Buffer;
use crate::selection::{Selection, SelectionSet};

// ── State parsing ─────────────────────────────────────────────────────────────

/// Parse a marker-annotated string into `(Buffer, SelectionSet)`.
///
/// The markers are stripped from the returned buffer. Panics with a
/// descriptive message if the string contains no cursor markers, or if a
/// marker is malformed (e.g. a `#[` with no matching `]#`).
pub(crate) fn parse_state(input: &str) -> (Buffer, SelectionSet) {
    // We scan the input one char at a time, building up:
    //   - `text`:       the raw buffer content (markers removed)
    //   - `selections`: the parsed Selection values
    //
    // State machine:
    //   Normal         — outside any marker
    //   InSelection    — inside #[…]# but before the | cursor marker
    //   AfterCursor    — inside #[…]# after the | cursor marker
    //
    // `char_count` tracks how many chars we have written to `text` so far,
    // which is the char offset we need for selection anchors and heads.

    let mut text = String::with_capacity(input.len());
    let mut selections: Vec<Selection> = Vec::new();

    #[derive(Debug)]
    enum State {
        Normal,
        /// Anchor was recorded at `anchor_offset`; we have not yet seen `|`.
        InSelection { anchor_offset: usize },
        /// We have seen `|` at `head_offset`; waiting for `]#`.
        AfterCursor { anchor_offset: usize, head_offset: usize },
    }

    let mut state = State::Normal;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match (&state, ch) {
            // ── Opening `#[` ──────────────────────────────────────────────
            (State::Normal, '#') if chars.peek() == Some(&'[') => {
                chars.next(); // consume '['
                let anchor = char_count(&text);
                state = State::InSelection { anchor_offset: anchor };
            }

            // ── Cursor `|` outside a selection → collapsed cursor ─────────
            (State::Normal, '|') => {
                let pos = char_count(&text);
                selections.push(Selection::cursor(pos));
            }

            // ── Cursor `|` inside a selection → marks the head ───────────
            (State::InSelection { anchor_offset }, '|') => {
                let head = char_count(&text);
                state = State::AfterCursor {
                    anchor_offset: *anchor_offset,
                    head_offset: head,
                };
            }

            // ── Closing `]#` ──────────────────────────────────────────────
            (State::AfterCursor { anchor_offset, head_offset }, ']')
                if chars.peek() == Some(&'#') =>
            {
                chars.next(); // consume '#'
                // Detect direction:
                //   Forward  — `|` appeared AFTER some text inside `#[…]#`,
                //               so anchor_offset < head_offset.
                //               anchor = anchor_offset, head = head_offset.
                //
                //   Backward — `|` appeared IMMEDIATELY after `#[` (no text
                //               between them), so anchor_offset == head_offset.
                //               In this case the text between `|` and `]#` is
                //               the selected region *after* the cursor, and
                //               the anchor sits at the current char position.
                //               anchor = current char_count, head = head_offset.
                let (anchor, head) = if *anchor_offset == *head_offset {
                    (char_count(&text), *head_offset)
                } else {
                    (*anchor_offset, *head_offset)
                };
                selections.push(Selection::new(anchor, head));
                state = State::Normal;
            }

            // ── Guard: `]` not followed by `#` is literal text ───────────
            (_, ']') => {
                text.push(']');
            }

            // ── Guard: `#` not followed by `[` is literal text ───────────
            (_, '#') => {
                text.push('#');
            }

            // ── Regular character — append to buffer text ─────────────────
            (_, c) => {
                text.push(c);
            }
        }
    }

    // Validate that the markers were properly closed.
    match state {
        State::InSelection { .. } => panic!(
            "parse_state: unterminated `#[` in input: {:?}\n\
             Did you forget the `|` cursor marker and `]#`?",
            input
        ),
        State::AfterCursor { .. } => panic!(
            "parse_state: `|` cursor marker without closing `]#` in input: {:?}",
            input
        ),
        State::Normal => {}
    }

    assert!(
        !selections.is_empty(),
        "parse_state: no cursor markers found in input: {:?}\n\
         Add at least one `|` or `#[...|...]#` marker.",
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
    let text = buf.to_string();
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();

    // Build a lookup: char_offset → what markers to insert before this char.
    // We use a `Vec` of vecs indexed by char position, plus a special slot
    // at index `n` for markers that appear after the last character (e.g. a
    // cursor at EOF).
    let mut markers: Vec<Vec<&'static str>> = vec![vec![]; n + 1];

    for sel in sels.iter_sorted() {
        if sel.is_cursor() {
            markers[sel.head].push("|");
        } else if sel.anchor <= sel.head {
            // Forward selection: #[...pre-cursor...|cursor-char]#rest
            //
            // Helix's cursor is *inclusive* — it sits ON the character at
            // `head`.  So `]#` goes one position past `head`, making the
            // cursor character visually appear between `|` and `]#`.
            markers[sel.anchor].push("#[");
            markers[sel.head].push("|");
            markers[sel.head + 1].push("]#");
        } else {
            // Backward selection: anchor > head.
            // Format: #[|...anchor_text...]#  — | at head (start), ]# at anchor (end).
            markers[sel.head].push("#[");
            markers[sel.head].push("|");
            markers[sel.end()].push("]#");
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
///     "|hello",                                    // initial state
///     |(buf, sels)| delete_char_forward(buf, sels), // operation
///     "|ello",                                     // expected state
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
    fn parse_collapsed_cursor_at_start() {
        let (buf, sels) = parse_state("|hello");
        assert_eq!(buf.to_string(), "hello");
        assert_eq!(sels.len(), 1);
        let s = sels.primary();
        assert!(s.is_cursor());
        assert_eq!(s.head, 0);
    }

    #[test]
    fn parse_collapsed_cursor_at_end() {
        let (buf, sels) = parse_state("hello|");
        assert_eq!(buf.to_string(), "hello");
        assert_eq!(sels.primary().head, 5);
    }

    #[test]
    fn parse_collapsed_cursor_in_middle() {
        let (buf, sels) = parse_state("hel|lo");
        assert_eq!(buf.to_string(), "hello");
        assert_eq!(sels.primary().head, 3);
    }

    #[test]
    fn parse_forward_selection() {
        // "#[hel|lo]# world" — anchor=0 (at #[), head=3 (at |).
        //
        // "lo" and " world" are both real buffer content — nothing is
        // discarded. The DSL is permissive: any chars between | and ]# go
        // into the buffer just like chars outside the markers. They represent
        // selected text after the cursor that the test author chose to show
        // inside the brackets for readability.
        //
        // The canonical serializer always places ]# at head+1, showing only
        // the single cursor character between | and ]#, so it would emit
        // "#[hel|l]#o world" for this selection. Both forms parse identically:
        // anchor=0, head=3, buffer="hello world".
        let (buf, sels) = parse_state("#[hel|lo]# world");
        assert_eq!(buf.to_string(), "hello world");
        let s = sels.primary();
        assert_eq!(s.anchor, 0);
        assert_eq!(s.head, 3);
    }

    #[test]
    fn parse_backward_selection() {
        // #[|hel]#lo → anchor=3, head=0
        let (buf, sels) = parse_state("#[|hel]#lo");
        assert_eq!(buf.to_string(), "hello");
        let s = sels.primary();
        assert_eq!(s.anchor, 3);
        assert_eq!(s.head, 0);
    }

    #[test]
    fn parse_selection_near_end_of_buffer() {
        // The ]# marker is the last thing in the input string, so the
        // selection extends to the end of the annotated text (but not
        // the end of the buffer — 'e' at offset 7 is unselected).
        let (buf, sels) = parse_state("hi #[the|re]#");
        assert_eq!(buf.to_string(), "hi there");
        let s = sels.primary();
        assert_eq!(s.anchor, 3);
        assert_eq!(s.head, 6);
    }

    #[test]
    fn parse_two_collapsed_cursors() {
        let (buf, sels) = parse_state("|foo| bar");
        assert_eq!(buf.to_string(), "foo bar");
        assert_eq!(sels.len(), 2);
        assert_eq!(sels.iter_sorted().next().unwrap().head, 0);
        assert_eq!(sels.iter_sorted().nth(1).unwrap().head, 3);
    }

    #[test]
    fn parse_two_forward_selections() {
        let (buf, sels) = parse_state("#[a|bc]# #[d|ef]#");
        assert_eq!(buf.to_string(), "abc def");
        assert_eq!(sels.len(), 2);
        let mut it = sels.iter_sorted();
        let s0 = it.next().unwrap();
        let s1 = it.next().unwrap();
        assert_eq!((s0.anchor, s0.head), (0, 1));
        assert_eq!((s1.anchor, s1.head), (4, 5));
    }

    #[test]
    fn parse_cursor_on_unicode_char() {
        // "é" is U+00E9, a single Unicode scalar value (1 char).
        let (buf, sels) = parse_state("caf|é");
        assert_eq!(buf.to_string(), "café");
        assert_eq!(sels.primary().head, 3);
    }

    #[test]
    fn parse_empty_selection_collapsed_at_zero() {
        let (buf, sels) = parse_state("|");
        assert_eq!(buf.to_string(), "");
        assert_eq!(sels.primary().head, 0);
    }

    // ── serialize_state ───────────────────────────────────────────────────────

    #[test]
    fn serialize_collapsed_cursor_at_start() {
        let buf = Buffer::from_str("hello");
        let sels = SelectionSet::single(Selection::cursor(0));
        assert_eq!(serialize_state(&buf, &sels), "|hello");
    }

    #[test]
    fn serialize_collapsed_cursor_at_end() {
        let buf = Buffer::from_str("hello");
        let sels = SelectionSet::single(Selection::cursor(5));
        assert_eq!(serialize_state(&buf, &sels), "hello|");
    }

    #[test]
    fn serialize_backward_selection() {
        // anchor=3, head=0 → cursor ON 'h' (char 0), anchor at offset 3.
        // #[ and | both at 0, ]# at anchor=3.
        let buf = Buffer::from_str("hello");
        let sels = SelectionSet::single(Selection::new(3, 0));
        assert_eq!(serialize_state(&buf, &sels), "#[|hel]#lo");
    }

    #[test]
    fn serialize_forward_selection() {
        // anchor=0, head=3 → cursor ON 'l' (char 3).
        // #[ at 0, | at 3, ]# at 4 (one past cursor char).
        let buf = Buffer::from_str("hello world");
        let sels = SelectionSet::single(Selection::new(0, 3));
        assert_eq!(serialize_state(&buf, &sels), "#[hel|l]#o world");
    }

    // ── Round-trip ────────────────────────────────────────────────────────────

    fn round_trip(s: &str) -> String {
        let (buf, sels) = parse_state(s);
        serialize_state(&buf, &sels)
    }

    #[test]
    fn roundtrip_collapsed_cursor() {
        assert_eq!(round_trip("|hello"), "|hello");
        assert_eq!(round_trip("hello|"), "hello|");
        assert_eq!(round_trip("hel|lo"), "hel|lo");
    }

    #[test]
    fn roundtrip_forward_selection() {
        // Canonical form: ]# sits one past the cursor char.
        // "#[hel|lo]# world" is a valid *input* (parses to anchor=0, head=3)
        // but is not canonical — its round-trip output is "#[hel|l]#o world".
        assert_eq!(round_trip("#[hel|l]#o world"), "#[hel|l]#o world");
    }

    #[test]
    fn roundtrip_backward_selection() {
        assert_eq!(round_trip("#[|hel]#lo"), "#[|hel]#lo");
    }

    #[test]
    fn roundtrip_two_cursors() {
        assert_eq!(round_trip("|foo| bar"), "|foo| bar");
    }

    #[test]
    fn roundtrip_empty_buffer_cursor() {
        assert_eq!(round_trip("|"), "|");
    }
}
