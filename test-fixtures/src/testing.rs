use hume_editing::changeset::ChangeSet;
use hume_editing::edit::Edited;
use hume_editing::selection::{Selection, SelectionSet};
use hume_editing::state::EditState;
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
///
/// Markers sit on grapheme-cluster boundaries and the head and anchor are the
/// starts of the clusters they wrap: `a-[e\u{301}]>b\n` is a cursor on the
/// whole accented letter. A marker inside a cluster panics.
use hume_editing::text::BufferText;
use hume_rope::cluster::ClusterStart;
use hume_rope::offset::CharOffset;

// ── IntoTestResult ────────────────────────────────────────────────────────────

/// Convert a command's return value into `(BufferText, SelectionSet)` for
/// assertion purposes.
///
/// Commands have two distinct signature families:
///
/// - **Non-mutating** (motions, text objects, selection commands): take an
///   `EditState`, return one for the same text.
/// - **Mutating** (edits): take an `EditState`, return `Edited`, whose text is
///   the edited one.
///
/// This trait lets `assert_state!` accept both families without change, plus
/// the bare pairs a test helper may build.
pub trait IntoTestResult {
    fn into_test_result(self) -> (BufferText, SelectionSet);
}

/// `(BufferText, SelectionSet)` pair — emitted by internal helpers that don't produce a `ChangeSet`.
impl IntoTestResult for (BufferText, SelectionSet) {
    fn into_test_result(self) -> (BufferText, SelectionSet) {
        self
    }
}

/// Edit commands on the typed model.
impl IntoTestResult for Edited {
    fn into_test_result(self) -> (BufferText, SelectionSet) {
        let (text, sels, _) = parts(self);
        (text, sels)
    }
}

/// Motions, text objects and selection commands on the typed model.
impl IntoTestResult for EditState {
    fn into_test_result(self) -> (BufferText, SelectionSet) {
        (self.text().clone(), self.into_selections())
    }
}

/// `sels` paired with `text`, the input an edit command takes.
pub fn state(text: BufferText, sels: SelectionSet) -> EditState {
    EditState::bind(&text, sels)
}

/// An edit's new text, selections and changeset.
pub fn parts(edited: Edited) -> (BufferText, SelectionSet, ChangeSet) {
    let (state, cs) = edited.into_parts();
    (state.text().clone(), state.into_selections(), cs)
}

// ── State parsing ─────────────────────────────────────────────────────────────

/// Parse a marker-annotated string into `(BufferText, SelectionSet)`. See
/// [`hume_editing::marked::parse`] for the notation and what panics.
pub fn parse_state(input: &str) -> (BufferText, SelectionSet) {
    let state = hume_editing::marked::parse(input);
    (state.text().clone(), state.into_selections())
}

// ── Typed positions from char offsets ─────────────────────────────────────────

/// The cluster starting at char `n` of `text`. Panics when `n` is not a
/// cluster start, so a test's literal offset names the position it means.
pub fn at(text: &BufferText, n: usize) -> ClusterStart {
    let pos = text.snap(CharOffset::new(n));
    assert_eq!(
        pos.offset(),
        CharOffset::new(n),
        "char {n} is not a cluster start"
    );
    pos
}

/// A selection of `text` from char `anchor` to char `head`, both cluster
/// starts.
pub fn sel(text: &BufferText, anchor: usize, head: usize) -> Selection {
    Selection::new(at(text, anchor), at(text, head))
}

/// A cursor on the cluster starting at char `n` of `text`.
pub fn cursor(text: &BufferText, n: usize) -> Selection {
    Selection::cursor(at(text, n))
}

/// `selections` as a set for `text`, the one at `primary` primary; sorted and
/// merged like every set.
pub fn set(text: &BufferText, selections: Vec<Selection>, primary: usize) -> SelectionSet {
    EditState::at_text_start(text.clone())
        .with_selections(selections, primary)
        .into_selections()
}

/// One selection as a set for `text`.
pub fn single(text: &BufferText, selection: Selection) -> SelectionSet {
    set(text, vec![selection], 0)
}

/// `(BufferText, SelectionSet)` in marker notation: the inverse of
/// [`parse_state`], so assertion diffs show markers rather than offsets.
pub fn serialize_state(text: &BufferText, sels: &SelectionSet) -> String {
    hume_editing::marked::render(hume_editing::selection::EditView::bind(text, sels))
}

// ── Assertion macro ───────────────────────────────────────────────────────────

/// Assert that applying `$op` to the state described by `$initial` produces
/// the state described by `$expected`.
///
/// Both `$initial` and `$expected` are marker-annotated strings (see module
/// docs for the format). `$op` is a closure that takes `(BufferText, SelectionSet)`
/// and returns anything [`IntoTestResult`] accepts: `Edited` for an edit,
/// `EditState` for a motion or selection command.
///
/// Both return types are handled automatically via [`IntoTestResult`].
///
/// # Example
///
/// ```text
/// // Edit command:
/// assert_state!(
///     "-[h]>ello\n",
///     |(text, sels)| delete_char_forward(state(text, sels)),
///     "-[e]>llo\n",
/// );
///
/// // Motion command:
/// assert_state!(
///     "-[h]>ello\n",
///     |(text, sels)| cmd_move_right(state(text, sels), 1, MotionMode::Move),
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

        let (text, sels) = parse_state($initial);
        let (result_text, result_sels) =
            $crate::testing::IntoTestResult::into_test_result($op((text, sels)));
        hume_editing::selection::EditView::bind(&result_text, &result_sels)
            .check()
            .expect("the command's selections fit its text");
        let (expected_text, expected_sels) = parse_state($expected);

        assert_eq!(
            serialize_state(&result_text, &result_sels),
            serialize_state(&expected_text, &expected_sels),
        );
    }};
}
