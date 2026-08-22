use hume_editing::changeset::ChangeSet;
use hume_editing::selection::{Selection, SelectionSet};
/// Test DSL for HUME editing operations.
///
/// A compact, human-readable string format for editor state (buffer content
/// + selections) inline in test source.
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
/// The cursor is *inclusive* — it sits on the head character, not between
/// characters. The text between `[` and `]` is exactly the selected text
/// (anchor and head both included); `-` always marks anchor, `>`/`<` always
/// marks head, and the arrow direction shows which way the selection faces.
/// Multiple selections in one string: `-[he]>llo -[wor]>ld\n`
use hume_editing::text::Text;

// ── IntoTestResult ────────────────────────────────────────────────────────────

/// Convert a command's return value into `(Text, SelectionSet)` for
/// assertion purposes.
///
/// Commands have two distinct signature families:
///
/// - **Non-mutating** (motions, text objects, selection commands): take
///   `&Text`, return `SelectionSet`. The buffer is unchanged — the macro
///   provides its clone via `original_buf`.
/// - **Mutating** (edits): take `Text` by value, return
///   `(Text, SelectionSet, ChangeSet)`. The returned buffer
///   is the edited one — `original_buf` is ignored.
///
/// This trait lets `assert_state!` accept both families without change.
pub trait IntoTestResult {
    fn into_test_result(self, original_buf: Text) -> (Text, SelectionSet);
}

/// Non-mutating commands return only the new `SelectionSet`.
/// The buffer didn't change, so we pair it with the caller's clone.
impl IntoTestResult for SelectionSet {
    fn into_test_result(self, original_buf: Text) -> (Text, SelectionSet) {
        (original_buf, self)
    }
}

/// `(Text, SelectionSet)` pair — emitted by internal helpers that don't produce a `ChangeSet`.
impl IntoTestResult for (Text, SelectionSet) {
    fn into_test_result(self, _original_buf: Text) -> (Text, SelectionSet) {
        self
    }
}

/// Standard edit commands.
impl IntoTestResult for (Text, SelectionSet, ChangeSet) {
    fn into_test_result(self, _original_buf: Text) -> (Text, SelectionSet) {
        (self.0, self.1)
    }
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

// ── State parsing ─────────────────────────────────────────────────────────────

/// Parse a marker-annotated string into `(Text, SelectionSet)`.
///
/// The markers are stripped from the returned buffer. Panics with a
/// descriptive message if the string contains no selection markers, or if a
/// marker is malformed (e.g. a `-[` with no matching `]>`).
pub fn parse_state(input: &str) -> (Text, SelectionSet) {
    // Single pass, tracking whether we're inside `-[…]>` or `<[…]-` (see
    // `State` below). Any char not starting one of the four two-char tokens
    // (recognised by peeking one char ahead) is literal text.

    let mut text = String::with_capacity(input.len());
    let mut selections: Vec<Selection> = Vec::new();

    #[derive(Debug)]
    enum State {
        Normal,
        /// Inside `-[…]>`: anchor was recorded at `anchor_offset`.
        InForward {
            anchor_offset: usize,
        },
        /// Inside `<[…]-`: head was recorded at `head_offset`.
        InBackward {
            head_offset: usize,
        },
    }

    let mut state = State::Normal;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match (&state, ch) {
            // ── Open forward: `-[` ────────────────────────────────────────
            (State::Normal, '-') if chars.peek() == Some(&'[') => {
                chars.next(); // consume '['
                state = State::InForward {
                    anchor_offset: char_count(&text),
                };
            }

            // ── Open backward: `<[` ───────────────────────────────────────
            (State::Normal, '<') if chars.peek() == Some(&'[') => {
                chars.next(); // consume '['
                state = State::InBackward {
                    head_offset: char_count(&text),
                };
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

    let buf = Text::from(text.as_str());
    let sel_set = SelectionSet::from_vec(selections, 0);
    (buf, sel_set)
}

/// Serialize `(Text, SelectionSet)` back to the marker format.
///
/// This is the inverse of `parse_state`. It is used in assertions so that
/// diffs show the annotated marker text rather than raw char offsets.
pub fn serialize_state(buf: &Text, sels: &SelectionSet) -> String {
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
        if sel.anchor() <= sel.head() {
            // Forward selection (including cursor where anchor == head).
            // `-[` at anchor, `]>` one past head.
            markers[sel.anchor()].push("-[");
            markers[(sel.head() + 1).min(n)].push("]>");
        } else {
            // Backward selection (anchor > head).
            // `<[` at head, `]-` one past anchor.
            markers[sel.head()].push("<[");
            markers[(sel.anchor() + 1).min(n)].push("]-");
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
/// docs for the format). `$op` is a closure that takes `(Text, SelectionSet)`
/// and returns either:
/// - `(Text, SelectionSet, ChangeSet[, Vec<String>])` — for edit commands
///   that modify the buffer, or
/// - `SelectionSet` — for motion/selection commands that only move cursors.
///
/// Both return types are handled automatically via [`IntoTestResult`].
///
/// # Example
///
/// ```text
/// // Edit command (returns buffer + sels + changeset):
/// assert_state!(
///     "-[h]>ello\n",
///     |(buf, sels)| delete_char_forward(buf, sels),
///     "-[e]>llo\n",
/// );
///
/// // Motion command (returns SelectionSet only):
/// assert_state!(
///     "-[h]>ello\n",
///     |(buf, sels)| cmd_move_right(&buf, sels, 1, MotionMode::Move),
///     "h-[e]>llo\n",
/// );
/// ```
///
/// On failure the error message shows both sides in marker format, making it
/// immediately obvious what went wrong.
#[macro_export]
macro_rules! assert_state {
    ($initial:expr, $op:expr, $expected:expr) => {{
        use pretty_assertions::assert_eq;
        use $crate::testing::{parse_state, serialize_state};

        let (buf, sels) = parse_state($initial);
        // Clone before the op: non-mutating commands return only `SelectionSet`,
        // so `IntoTestResult` re-pairs it with this clone. Mutating commands
        // return a new buffer and ignore the clone. Rope clones are O(log n).
        let buf_copy = buf.clone();
        let (result_buf, result_sels) =
            $crate::testing::IntoTestResult::into_test_result($op((buf, sels)), buf_copy);
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
mod tests;
