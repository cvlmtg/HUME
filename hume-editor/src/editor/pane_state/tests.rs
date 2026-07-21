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
