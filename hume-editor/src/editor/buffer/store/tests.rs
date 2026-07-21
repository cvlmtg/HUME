use super::*;
use hume_editing::selection::SelectionSet;
use hume_editing::text::Text;
use hume_engine::pipeline::{BufferId, EngineView};
use hume_engine::theme::Theme;

fn make_id(ev: &mut EngineView) -> BufferId {
    ev.buffers.insert(())
}

fn make_buf() -> Buffer {
    Buffer::new(Text::from("hello\n"), SelectionSet::default())
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
