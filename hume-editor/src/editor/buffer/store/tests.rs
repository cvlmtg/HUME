use super::*;
use hume_editing::text::BufferText;
use hume_engine::pipeline::{BufferId, EngineView};
use hume_engine::theme::Theme;

fn make_id(ev: &mut EngineView) -> BufferId {
    ev.buffers.insert(())
}

fn make_buf() -> Buffer {
    Buffer::at_start(BufferText::from("hello\n"))
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
fn close_removes_from_mru() {
    let (mut store, mut ev) = store_with_engine();
    let a = make_id(&mut ev);
    let b = make_id(&mut ev);
    store.open(a, make_buf());
    store.open(b, make_buf());
    // mru = [b, a] (each open seeds at the head; see `open`'s own doc).
    // Closing b must drop it from `mru` too, leaving a as the sole entry.
    store.close(b);
    assert_eq!(store.len(), 1);
    assert_eq!(store.mru_excluding(b), Some(a));
    assert_eq!(store.second_most_recent(), None);
}

#[test]
fn mru_excluding_none_with_single_buffer() {
    let (mut store, mut ev) = store_with_engine();
    let a = make_id(&mut ev);
    store.open(a, make_buf());
    assert_eq!(store.mru_excluding(a), None);
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
    // Each open seeds at the *head* of `mru` (see `open`'s own doc), so
    // later opens push earlier ones toward the tail: mru = [b, a], with a
    // (opened first) at the tail. Touch b to make it most recent instead.
    store.touch_mru(b);
    assert_eq!(store.second_most_recent(), Some(a));
}

/// `edit_seq` starts at 0 and only moves via the explicit bump. Nothing else
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
/// not a user edit: it must not advance `edit_seq`, or a `PasteStamp`
/// stamped by a capture would go stale just from the user glancing at
/// `:messages` between a kill and a paste.
#[test]
fn view_content_refresh_does_not_bump_edit_seq() {
    let (mut store, mut ev) = store_with_engine();
    let id = make_id(&mut ev);
    store.open(id, make_buf());
    let before = store.edit_seq();
    store.get_mut(id).set_view_content(
        BufferId::default(),
        &mut crate::editor::position_stores::DetachedStores::default().stores(),
        "refreshed\n",
    );
    assert_eq!(
        store.edit_seq(),
        before,
        "set_view_content is a system refresh, not a user edit: edit_seq must not move"
    );
}

/// `Buffer::reload_from_text` (`:e!`) is likewise a system refresh, not a
/// user edit, same rationale as `view_content_refresh_does_not_bump_edit_seq`.
#[test]
fn reload_from_text_does_not_bump_edit_seq() {
    let (mut store, mut ev) = store_with_engine();
    let id = make_id(&mut ev);
    store.open(id, make_buf());
    let before = store.edit_seq();
    let (mut stores, pane, stored_as) = crate::editor::position_stores::DetachedStores::with_pane(
        store.get(id),
        store.get(id).initial_sels(),
    );
    assert!(store.get_mut(id).reload_from_text(
        stored_as,
        &mut stores.stores(),
        BufferText::from("reloaded\n"),
        pane,
    ));
    assert_eq!(
        store.edit_seq(),
        before,
        "reload_from_text (:e!) is a system refresh, not a user edit: edit_seq must not move"
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

/// After a mutation, exactly the touched buffer is reported once, and a
/// second immediate call reports nothing, since the baseline already caught
/// up.
///
/// Without the `announced_generation` write in `take_text_changed`, the second
/// call would still return `[id]`.
#[test]
fn take_text_changed_reports_a_touched_buffer_once() {
    let (mut store, mut ev) = store_with_engine();
    let id = make_id(&mut ev);
    store.open(id, make_buf());
    store.get_mut(id).set_view_content(
        BufferId::default(),
        &mut crate::editor::position_stores::DetachedStores::default().stores(),
        "edited\n",
    );

    assert_eq!(store.take_text_changed(), vec![id]);
    assert_eq!(
        store.take_text_changed(),
        Vec::new(),
        "baseline must advance so a second call sees nothing new"
    );
}

/// Several mutations to the same buffer between two calls coalesce into one
/// report: the coalescing contract `on-text-changed` documents.
#[test]
fn take_text_changed_coalesces_multiple_mutations_into_one_report() {
    let (mut store, mut ev) = store_with_engine();
    let id = make_id(&mut ev);
    store.open(id, make_buf());
    store.get_mut(id).set_view_content(
        BufferId::default(),
        &mut crate::editor::position_stores::DetachedStores::default().stores(),
        "first\n",
    );
    store.get_mut(id).set_view_content(
        BufferId::default(),
        &mut crate::editor::position_stores::DetachedStores::default().stores(),
        "second\n",
    );
    store.get_mut(id).set_view_content(
        BufferId::default(),
        &mut crate::editor::position_stores::DetachedStores::default().stores(),
        "third\n",
    );

    assert_eq!(store.take_text_changed(), vec![id]);
}

/// Only a buffer that actually mutated is reported. A sibling buffer left
/// untouched must not appear.
#[test]
fn take_text_changed_ignores_untouched_siblings() {
    let (mut store, mut ev) = store_with_engine();
    let a = make_id(&mut ev);
    let b = make_id(&mut ev);
    store.open(a, make_buf());
    store.open(b, make_buf());
    store.get_mut(b).set_view_content(
        BufferId::default(),
        &mut crate::editor::position_stores::DetachedStores::default().stores(),
        "only b\n",
    );

    assert_eq!(store.take_text_changed(), vec![b]);
}
