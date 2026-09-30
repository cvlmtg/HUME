use super::*;
use crate::editor::buffer::Buffer;
use hume_editing::text::BufferText;
use hume_engine::pipeline::BufferId;
use test_fixtures::testing::{cursor, single};

#[test]
fn fresh_from_buf_seeds_initial_sels() {
    let buf = Buffer::scratch();
    let expected = test_fixtures::testing::serialize_state(buf.text(), &buf.initial_sels());
    let state = fresh_from_buf(&buf);
    assert_eq!(
        test_fixtures::testing::serialize_state(buf.text(), &state.selections),
        expected
    );
    assert!(state.search_cursor.match_count.is_none());
}

#[test]
fn fresh_from_buf_seeds_stable_initial_sels_across_promotion() {
    // A pane that first views a buffer *after* undo-levels promotion has run
    // must still see the buffer's true open-time selection, not a later
    // revision's post-edit cursor. If enforce_undo_levels overwrote the
    // root's `forward` transaction on promotion, `initial_sels()` and this
    // seed would both return that later selection.
    use hume_ops::edit::insert_char;

    let mut buf = Buffer::at_start(BufferText::from("hello\n"));
    let initial_head = |buf: &Buffer, sels: &SelectionSet| {
        hume_editing::selection::EditView::bind(buf.text(), sels)
            .primary()
            .selection()
    };
    let expected = initial_head(&buf, &buf.initial_sels());
    buf.set_undo_levels(1);

    let start = buf.initial_sels();
    let (sels, _cs) = buf.apply_edit(
        BufferId::default(),
        &mut crate::editor::position_stores::DetachedStores::default().stores(),
        start,
        |s| insert_char(s, 'x'),
    );
    // Second edit pushes the tree past cap 1, promoting the first edit into root.
    buf.apply_edit(
        BufferId::default(),
        &mut crate::editor::position_stores::DetachedStores::default().stores(),
        sels,
        |s| insert_char(s, 'y'),
    );

    let state = fresh_from_buf(&buf);
    assert_eq!(initial_head(&buf, &state.selections), expected);
}

// ── reveal ──────────────────────────────────────────────────────────────────

fn accented_text() -> BufferText {
    BufferText::from("ae\u{301}b\n")
}

#[test]
fn set_selections_raises_no_reveal_when_the_head_stays() {
    let text = accented_text();
    let mut state = fresh_from_buf(&Buffer::at_start(text.clone()));
    state.set_selections(single(&text, cursor(&text, 1)), &text);
    state.reveal_pending = false;
    state.set_selections(single(&text, cursor(&text, 1)), &text);
    assert!(!state.reveal_pending);
}

#[test]
fn set_selections_raises_a_reveal_when_the_head_moves() {
    let text = accented_text();
    let mut state = fresh_from_buf(&Buffer::at_start(text.clone()));
    state.set_selections(single(&text, cursor(&text, 1)), &text);
    state.reveal_pending = false;
    state.set_selections(single(&text, cursor(&text, 3)), &text);
    assert!(state.reveal_pending);
}
