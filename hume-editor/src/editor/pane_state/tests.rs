use super::*;
use crate::editor::tests::co;

#[test]
fn pane_buffer_state_default_is_valid() {
    use hume_editing::selection::Selection;
    let state = PaneBufferState::default();
    assert_eq!(state.selections.primary(), Selection::collapsed(co(0)));
    assert!(state.search_cursor.match_count.is_none());
}

#[test]
fn fresh_from_buf_seeds_initial_sels() {
    use crate::editor::buffer::Buffer;
    let buf = Buffer::scratch();
    let expected = buf.initial_sels();
    let state = fresh_from_buf(&buf);
    assert_eq!(state.selections, expected);
    assert!(state.search_cursor.match_count.is_none());
}

#[test]
fn fresh_from_buf_seeds_stable_initial_sels_across_promotion() {
    // A pane that first views a buffer *after* undo-levels promotion has run
    // must still see the buffer's true open-time selection, not a later
    // revision's post-edit cursor. If enforce_undo_levels overwrote the
    // root's `forward` transaction on promotion, `initial_sels()` and this
    // seed would both return that later selection.
    use crate::editor::buffer::Buffer;
    use hume_editing::text::BufferText;
    use hume_ops::edit::insert_char;

    let mut buf = Buffer::new(BufferText::from("hello\n"), SelectionSet::default());
    let expected = buf.initial_sels();
    buf.set_undo_levels(1);

    let (sels, _cs) = buf.apply_edit(SelectionSet::default(), |b, s| insert_char(b, s, 'x'));
    // Second edit pushes the tree past cap 1, promoting the first edit into root.
    buf.apply_edit(sels, |b, s| insert_char(b, s, 'y'));

    let state = fresh_from_buf(&buf);
    assert_eq!(state.selections, expected);
}

// ── cluster alignment ─────────────────────────────────────────────────────

fn accented_text() -> hume_editing::text::BufferText {
    hume_editing::text::BufferText::from("ae\u{301}b\n")
}

#[test]
fn set_selections_snaps_a_head_on_a_combining_mark_to_its_cluster_start() {
    let mut state = PaneBufferState::default();
    state.set_selections(
        SelectionSet::single(hume_editing::selection::Selection::collapsed(co(2))),
        &accented_text(),
    );
    assert_eq!(state.selections.primary().head(), co(1));
}

#[test]
fn restore_selections_compares_the_snapped_head_against_the_old_one() {
    let mut state = PaneBufferState {
        reveal_pending: false,
        ..PaneBufferState::default()
    };
    state.restore_selections(
        SelectionSet::single(hume_editing::selection::Selection::collapsed(co(2))),
        co(1),
        &accented_text(),
    );
    assert!(!state.reveal_pending);
}

#[test]
fn fresh_from_buf_snaps_the_initial_selection() {
    use crate::editor::buffer::Buffer;
    let buf = Buffer::new(
        accented_text(),
        SelectionSet::single(hume_editing::selection::Selection::collapsed(co(2))),
    );
    assert_eq!(fresh_from_buf(&buf).selections.primary().head(), co(1));
}
