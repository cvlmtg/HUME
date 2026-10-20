//! `doc_ops`: an edit applied from selections the caller gives rather than
//! the pane's own.

use super::*;
use crate::editor::doc_ops;
use hume_editing::selection::SelectionSet;
use hume_ops::edit::make_text_uppercase;

/// Uppercase what `sels` covers in `bid`, acting through the focused pane.
fn uppercase_from(ed: &mut Editor, bid: BufferId, sels: SelectionSet) {
    let pid = ed.view.focused_pane();
    let (buffers, mut stores) = ed.state.edit_parts();
    doc_ops::apply_doc_edit_on(buffers, &mut stores, pid, bid, sels, make_text_uppercase)
        .expect("no session is open on the buffer");
}

#[test]
fn an_edit_from_given_selections_lands_there_and_undo_restores_them() {
    let mut ed = editor_from("-[ab]> cd\n");
    let bid = ed.focused_buffer_id();
    let given = ed.current_selections().clone();
    ed.feed_key(key('w'));
    assert_ne!(state(&ed), "-[ab]> cd\n", "setup: the selection moved");

    uppercase_from(&mut ed, bid, given);
    assert_eq!(state(&ed), "-[AB]> cd\n");

    ed.feed_key(key('u'));
    assert_eq!(state(&ed), "-[ab]> cd\n");
}

#[test]
fn an_edit_from_given_selections_lands_when_the_pane_shows_another_buffer() {
    let mut ed = editor_from("-[ab]> cd\n");
    let bid = ed.focused_buffer_id();
    let given = ed.current_selections().clone();
    open_other_buffer(&mut ed, "x\n");

    uppercase_from(&mut ed, bid, given);
    assert_eq!(ed.state.buffers.get(bid).text().to_string(), "AB cd\n");

    ed.switch_to_buffer_with_jump(FocusedPane::current(&ed.view), bid);
    assert_eq!(state(&ed), "-[AB]> cd\n");
}
