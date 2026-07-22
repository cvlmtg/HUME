use super::*;

#[test]
fn pane_buffer_state_default_is_valid() {
    use hume_editing::selection::Selection;
    let state = PaneBufferState::default();
    assert_eq!(state.selections.primary(), Selection::collapsed(0));
    assert!(state.edit_group.is_none());
    assert!(state.paste_group.is_none());
    assert!(state.search_cursor.match_count.is_none());
}

#[test]
fn pane_transient_default_is_empty() {
    let t = PaneTransient::default();
    assert!(t.pre_search_sels.is_none());
    assert!(t.pre_select_sels.is_none());
    assert!(!t.search_extend);
}

#[test]
fn fresh_from_buf_seeds_initial_sels() {
    use crate::editor::buffer::Buffer;
    let buf = Buffer::scratch();
    let expected = buf.initial_sels();
    let state = fresh_from_buf(&buf);
    assert_eq!(state.selections, expected);
    assert!(state.edit_group.is_none());
    assert!(state.paste_group.is_none());
    assert!(state.search_cursor.match_count.is_none());
}

#[test]
fn fresh_from_buf_seeds_stable_initial_sels_across_promotion() {
    // A pane that first views a buffer *after* undo-levels promotion has run
    // must still see the buffer's true open-time selection, not a later
    // revision's post-edit cursor.
    // Fail oracle: if enforce_undo_levels overwrote the root's `forward`
    // transaction on promotion, `initial_sels()` (and this seed) would
    // return the promoted revision's post-edit selection instead.
    use crate::editor::buffer::Buffer;
    use crate::ops::edit::insert_char;

    let mut buf = Buffer::new(Text::from("hello\n"), SelectionSet::default());
    let expected = buf.initial_sels();
    buf.set_undo_levels(1);

    let (sels, _cs) = buf.apply_edit(SelectionSet::default(), |b, s| insert_char(b, s, 'x'));
    // Second edit pushes the tree past cap 1, promoting the first edit into root.
    buf.apply_edit(sels, |b, s| insert_char(b, s, 'y'));

    let state = fresh_from_buf(&buf);
    assert_eq!(state.selections, expected);
}
