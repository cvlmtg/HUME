use super::*;
use hume_editing::selection::SelectionSet;
use hume_editing::text::BufferText;
use hume_engine::pipeline::{BufferId, EngineView};
use hume_engine::theme::Theme;

fn make_id(ev: &mut EngineView) -> BufferId {
    ev.buffers.insert(())
}

fn make_buf() -> Buffer {
    Buffer::new(BufferText::from("hello\n"), SelectionSet::default())
}

fn store_with_engine() -> (BufferStore, EngineView) {
    let ev = EngineView::new(Theme::default());
    (BufferStore::new(), ev)
}

#[test]
fn open_and_get() {
    let (mut store, mut ev) = store_with_engine();
    let id = make_id(&mut ev);
    store.open(id, make_buf());
    assert_eq!(store.get(id).text().to_string(), "hello\n");
    assert_eq!(store.len(), 1);
}

#[test]
fn close_returns_mru_replacement() {
    let (mut store, mut ev) = store_with_engine();
    let a = make_id(&mut ev);
    let b = make_id(&mut ev);
    store.open(a, make_buf());
    store.open(b, make_buf());
    // b is MRU tail (most recent). closing b should suggest a.
    let replacement = store.close(b);
    assert_eq!(replacement, Some(a));
    assert_eq!(store.len(), 1);
}

#[test]
fn close_last_returns_none() {
    let (mut store, mut ev) = store_with_engine();
    let a = make_id(&mut ev);
    store.open(a, make_buf());
    assert_eq!(store.close(a), None);
}

#[test]
fn next_and_prev_wrap() {
    let (mut store, mut ev) = store_with_engine();
    let a = make_id(&mut ev);
    let b = make_id(&mut ev);
    let c = make_id(&mut ev);
    store.open(a, make_buf());
    store.open(b, make_buf());
    store.open(c, make_buf());
    assert_eq!(store.next(c), a, "next wraps to start");
    assert_eq!(store.prev(a), c, "prev wraps to end");
    assert_eq!(store.next(a), b);
    assert_eq!(store.prev(c), b);
}

#[test]
fn find_by_path_dedup() {
    let (mut store, mut ev) = store_with_engine();
    let id = make_id(&mut ev);
    let mut buf = make_buf();
    buf.set_path(Some(std::path::PathBuf::from("/tmp/foo.txt")));
    store.open(id, buf);
    assert_eq!(store.find_by_path(Path::new("/tmp/foo.txt")), Some(id));
    assert_eq!(store.find_by_path(Path::new("/tmp/bar.txt")), None);
}

#[test]
fn touch_mru_promotes_to_tail() {
    let (mut store, mut ev) = store_with_engine();
    let a = make_id(&mut ev);
    let b = make_id(&mut ev);
    store.open(a, make_buf());
    store.open(b, make_buf());
    // b is MRU tail. Touch a to make it most recent.
    store.touch_mru(a);
    assert_eq!(store.mru_excluding(a), Some(b));
}

/// `edit_seq` starts at 0 and only moves via the explicit bump — nothing else
/// touches it (see `PasteStamp`, which relies on this for staleness checks).
#[test]
fn edit_seq_starts_at_zero_and_bumps_explicitly() {
    let mut store = BufferStore::new();
    assert_eq!(store.edit_seq(), 0);
    store.bump_edit_seq();
    assert_eq!(store.edit_seq(), 1);
    store.bump_edit_seq();
    assert_eq!(store.edit_seq(), 2);
}

/// `Buffer::set_view_content` (`:messages`/`:ls` refresh) is a system refresh,
/// not a user edit — it must not advance `edit_seq`, or a `PasteStamp`
/// stamped by a capture would go stale just from the user glancing at
/// `:messages` between a kill and a paste.
///
/// Fail oracle: route `set_view_content` through `BufferStore::bump_edit_seq`
/// (or through the `doc_ops` chokepoint) → this test's `assert_eq!` fails.
#[test]
fn view_content_refresh_does_not_bump_edit_seq() {
    let (mut store, mut ev) = store_with_engine();
    let id = make_id(&mut ev);
    store.open(id, make_buf());
    let before = store.edit_seq();
    store
        .get_mut(id)
        .set_view_content(BufferText::from("refreshed\n"));
    assert_eq!(
        store.edit_seq(),
        before,
        "set_view_content is a system refresh, not a user edit — edit_seq must not move"
    );
}

/// `Buffer::reload_from_text` (`:e!`) is likewise a system refresh, not a
/// user edit — same rationale as `view_content_refresh_does_not_bump_edit_seq`.
#[test]
fn reload_from_text_does_not_bump_edit_seq() {
    let (mut store, mut ev) = store_with_engine();
    let id = make_id(&mut ev);
    store.open(id, make_buf());
    let before = store.edit_seq();
    store.get_mut(id).reload_from_text(
        BufferText::from("reloaded\n"),
        SelectionSet::default(),
        SelectionSet::default(),
    );
    assert_eq!(
        store.edit_seq(),
        before,
        "reload_from_text (:e!) is a system refresh, not a user edit — edit_seq must not move"
    );
}

// ── take_text_changed ───────────────────────────────────────────────────────

#[test]
fn take_text_changed_reports_nothing_for_an_untouched_store() {
    let (mut store, mut ev) = store_with_engine();
    let id = make_id(&mut ev);
    store.open(id, make_buf());
    assert_eq!(store.take_text_changed(), Vec::new());
}

/// After a mutation, exactly the touched buffer is reported once — and a
/// second immediate call reports nothing, since the baseline already caught
/// up.
///
/// Fail oracle: drop the `announced_text_gen = text_gen` write in
/// `take_text_changed` → the second call still returns `[id]`.
#[test]
fn take_text_changed_reports_a_touched_buffer_once() {
    let (mut store, mut ev) = store_with_engine();
    let id = make_id(&mut ev);
    store.open(id, make_buf());
    store
        .get_mut(id)
        .set_view_content(BufferText::from("edited\n"));

    assert_eq!(store.take_text_changed(), vec![id]);
    assert_eq!(
        store.take_text_changed(),
        Vec::new(),
        "baseline must advance so a second call sees nothing new"
    );
}

/// Several mutations to the same buffer between two calls coalesce into one
/// report — the coalescing contract `on-text-changed` documents.
#[test]
fn take_text_changed_coalesces_multiple_mutations_into_one_report() {
    let (mut store, mut ev) = store_with_engine();
    let id = make_id(&mut ev);
    store.open(id, make_buf());
    store
        .get_mut(id)
        .set_view_content(BufferText::from("first\n"));
    store
        .get_mut(id)
        .set_view_content(BufferText::from("second\n"));
    store
        .get_mut(id)
        .set_view_content(BufferText::from("third\n"));

    assert_eq!(store.take_text_changed(), vec![id]);
}

/// Only a buffer that actually mutated is reported — a sibling buffer left
/// untouched must not appear.
#[test]
fn take_text_changed_ignores_untouched_siblings() {
    let (mut store, mut ev) = store_with_engine();
    let a = make_id(&mut ev);
    let b = make_id(&mut ev);
    store.open(a, make_buf());
    store.open(b, make_buf());
    store
        .get_mut(b)
        .set_view_content(BufferText::from("only b\n"));

    assert_eq!(store.take_text_changed(), vec![b]);
}
