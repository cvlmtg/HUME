use super::*;
use crate::editor::buffer::NewBuffer;
use crate::editor::buffer::store::BufferStore;
use hume_editing::text::BufferText;
use test_fixtures::testing::{cursor, single};

#[test]
fn fresh_from_buf_seeds_initial_sels() {
    let (store, id) = BufferStore::holding(NewBuffer::Scratch(BufferText::empty()));
    let buf = store.get(id);
    let expected = test_fixtures::testing::serialize_state(buf.text(), &buf.initial_sels());
    let state = fresh_from_buf(buf);
    assert_eq!(
        test_fixtures::testing::serialize_state(buf.text(), state.selections()),
        expected
    );
    assert!(state.search_cursor.match_count.is_none());
}

// ── reveal ──────────────────────────────────────────────────────────────────

fn accented_text() -> BufferText {
    BufferText::from("ae\u{301}b\n")
}

#[test]
fn set_selections_raises_no_reveal_when_the_head_stays() {
    let text = accented_text();
    let (store, id) = BufferStore::holding(NewBuffer::Scratch(text.clone()));
    let mut state = fresh_from_buf(store.get(id));
    state.set_selections(single(&text, cursor(&text, 1)), &text);
    state.reveal_pending = false;
    state.set_selections(single(&text, cursor(&text, 1)), &text);
    assert!(!state.reveal_pending);
}

#[test]
fn set_selections_raises_a_reveal_when_the_head_moves() {
    let text = accented_text();
    let (store, id) = BufferStore::holding(NewBuffer::Scratch(text.clone()));
    let mut state = fresh_from_buf(store.get(id));
    state.set_selections(single(&text, cursor(&text, 1)), &text);
    state.reveal_pending = false;
    state.set_selections(single(&text, cursor(&text, 3)), &text);
    assert!(state.reveal_pending);
}
