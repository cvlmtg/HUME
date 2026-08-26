// Fuzzy-picker panel: key interception (`handle_picker_key`), the open
// chokepoint (`picker::open_picker`), and the per-frame write side
// (`sync_picker_view`). Sessions are still constructed directly rather than
// through the `picker!` Steel builtin — see `tests/picker_steel.rs` for
// end-to-end coverage of the Steel surface itself.

use super::*;
use crate::editor::lsp::completion::{CompletionSession, StoredCompletionItem};
use crate::editor::picker::{self, PickerItem, PickerSession};
use crate::ui::picker_panel::panel_geometry;
use hume_engine::pipeline::RenderContext;
use hume_scripting::host::PickerOpts;
use steel::rvals::SteelVal;

fn marker(name: &str) -> SteelVal {
    SteelVal::StringV(name.into())
}

fn open_test_picker_with_callback(ed: &mut Editor, items: &[&str], callback: SteelVal) {
    let mut session = PickerSession::new(callback, PickerOpts::default());
    let picker_items: Vec<PickerItem> = items
        .iter()
        .map(|s| PickerItem {
            display: s.to_string(),
            payload: SteelVal::StringV((*s).into()),
        })
        .collect();
    session.push(picker_items);
    picker::open_picker(&mut ed.state, Some(&mut ed.lsp), session);
}

fn open_test_picker(ed: &mut Editor, items: &[&str]) {
    open_test_picker_with_callback(ed, items, marker("cb"));
}

fn open_test_picker_with_prompt(ed: &mut Editor, items: &[&str], prompt: &str) {
    let mut session = PickerSession::new(
        marker("cb"),
        PickerOpts {
            prompt: prompt.to_string(),
            ..Default::default()
        },
    );
    let picker_items: Vec<PickerItem> = items
        .iter()
        .map(|s| PickerItem {
            display: s.to_string(),
            payload: SteelVal::StringV((*s).into()),
        })
        .collect();
    session.push(picker_items);
    picker::open_picker(&mut ed.state, Some(&mut ed.lsp), session);
}

/// Runs the write-side pipeline (`prepare_frame`) at a given terminal size —
/// needed before any test that depends on `panel_geometry`/`last_pane_area`
/// (paging, scroll clamping, the synced view).
fn frame(ed: &mut Editor, width: u16, height: u16) {
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(width, height);
    ed.settle();
    ed.prepare_frame(&mut ctx);
}

fn payload_str(v: &SteelVal) -> &str {
    match v {
        SteelVal::StringV(s) => s.as_str(),
        other => panic!("expected StringV payload, got {other:?}"),
    }
}

fn callback_name(v: &SteelVal) -> &str {
    payload_str(v)
}

// ── Printables edit the query, not the buffer ──────────────────────────────

#[test]
fn printables_edit_query_not_the_buffer() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_picker(&mut ed, &["one", "two"]);
    ed.feed_key(key('z'));

    let picker = ed.state.config.picker.as_ref().expect("picker still open");
    assert_eq!(picker.query(), "z");
    assert_eq!(
        ed.doc().text().to_string(),
        "abc\n",
        "buffer must be untouched"
    );
}

// ── Selection movement ──────────────────────────────────────────────────────

#[test]
fn up_down_and_ctrl_p_n_move_selection() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_picker(&mut ed, &["a", "b", "c"]);
    frame(&mut ed, 60, 16);

    ed.feed_key(key_down());
    assert_eq!(ed.state.config.picker.as_ref().unwrap().selected(), 1);
    ed.feed_key(key_ctrl('n'));
    assert_eq!(ed.state.config.picker.as_ref().unwrap().selected(), 2);
    ed.feed_key(key_up());
    assert_eq!(ed.state.config.picker.as_ref().unwrap().selected(), 1);
    ed.feed_key(key_ctrl('p'));
    assert_eq!(ed.state.config.picker.as_ref().unwrap().selected(), 0);
}

#[test]
fn page_keys_move_by_panel_list_rows() {
    let mut ed = editor_from("-[a]>bc\n");
    let items: Vec<String> = (0..50).map(|i| format!("item{i}")).collect();
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    open_test_picker(&mut ed, &refs);
    frame(&mut ed, 60, 16);

    let geo = panel_geometry(ed.view.last_pane_area).expect("viable geometry at 60x16");
    ed.feed_key(key_pagedown());
    assert_eq!(
        ed.state.config.picker.as_ref().unwrap().selected(),
        geo.list_rows
    );
    ed.feed_key(key_pageup());
    assert_eq!(ed.state.config.picker.as_ref().unwrap().selected(), 0);
}

#[test]
fn page_keys_before_first_frame_are_safe_noops() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_picker(&mut ed, &["a", "b", "c"]);
    // No `frame()` call: `last_pane_area` is still `Rect::default()`.
    ed.feed_key(key_pagedown());
    assert_eq!(ed.state.config.picker.as_ref().unwrap().selected(), 0);
    ed.feed_key(key_pageup());
    assert_eq!(ed.state.config.picker.as_ref().unwrap().selected(), 0);
}

#[test]
fn half_page_keys_move_by_half_the_list_rows() {
    let mut ed = editor_from("-[a]>bc\n");
    let items: Vec<String> = (0..50).map(|i| format!("item{i}")).collect();
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    open_test_picker(&mut ed, &refs);
    frame(&mut ed, 60, 16);

    let geo = panel_geometry(ed.view.last_pane_area).expect("viable geometry at 60x16");
    ed.feed_key(key_ctrl('d'));
    assert_eq!(
        ed.state.config.picker.as_ref().unwrap().selected(),
        geo.list_rows.div_ceil(2)
    );
    ed.feed_key(key_ctrl('u'));
    assert_eq!(ed.state.config.picker.as_ref().unwrap().selected(), 0);
}

#[test]
fn half_page_keys_before_first_frame_are_safe_noops() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_picker(&mut ed, &["a", "b", "c"]);
    // No `frame()` call: `last_pane_area` is still `Rect::default()`.
    ed.feed_key(key_ctrl('d'));
    assert_eq!(ed.state.config.picker.as_ref().unwrap().selected(), 0);
    ed.feed_key(key_ctrl('u'));
    assert_eq!(ed.state.config.picker.as_ref().unwrap().selected(), 0);
}

#[test]
fn half_page_down_saturates_at_last_item_without_wrapping() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_picker(&mut ed, &["a", "b", "c"]);
    frame(&mut ed, 60, 16);

    ed.feed_key(key_ctrl('d'));
    ed.feed_key(key_ctrl('d'));
    ed.feed_key(key_ctrl('d'));
    assert_eq!(ed.state.config.picker.as_ref().unwrap().selected(), 2);
}

// ── Backspace ────────────────────────────────────────────────────────────────

#[test]
fn backspace_pops_full_grapheme() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_picker(&mut ed, &["one"]);
    ed.feed_key(key('o'));
    ed.feed_key(key('n'));
    ed.feed_key(key_backspace());
    assert_eq!(ed.state.config.picker.as_ref().unwrap().query(), "o");
}

#[test]
fn backspace_on_empty_keeps_picker_open() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_picker(&mut ed, &["one"]);
    ed.feed_key(key_backspace());
    assert!(
        ed.state.config.picker.is_some(),
        "backspace on an empty query must not close the picker"
    );
    assert_eq!(ed.state.config.picker.as_ref().unwrap().query(), "");
}

// ── Stray keys ───────────────────────────────────────────────────────────────

#[test]
fn stray_keys_are_consumed_and_ignored() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_picker(&mut ed, &["one"]);
    for stray in [
        key_left(),
        key_tab(),
        KeyEvent::new(KeyCode::Home, Modifiers::NONE),
    ] {
        ed.feed_key(stray);
    }
    assert!(
        ed.state.config.picker.is_some(),
        "stray keys must not close the picker"
    );
    assert_eq!(ed.state.config.picker.as_ref().unwrap().query(), "");
    assert_eq!(
        ed.doc().text().to_string(),
        "abc\n",
        "buffer must be untouched"
    );
}

// ── Enter / Esc — terminal actions ──────────────────────────────────────────

#[test]
fn enter_fires_on_select_with_payload_and_closes() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_picker_with_callback(&mut ed, &["one", "two"], marker("cb"));
    ed.feed_key(key_enter());

    assert!(
        ed.state.config.picker.is_none(),
        "picker must close on Enter"
    );
    assert_eq!(pending_calls(&ed).len(), 1);
    let (proc, args) = pending_calls(&ed)[0];
    assert_eq!(callback_name(proc), "cb");
    assert_eq!(args.len(), 1);
    assert_eq!(payload_str(&args[0]), "one", "top-ranked item's payload");

    // L4: keep interacting past the terminal action — Enter must not leave
    // the editor in a half-consistent state for the next keystroke.
    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "Xabc\n");
    assert_eq!(
        pending_calls(&ed).len(),
        1,
        "no second callback should have fired from unrelated typing"
    );
}

#[test]
fn esc_fires_false_and_closes() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_picker_with_callback(&mut ed, &["one"], marker("cb"));
    ed.feed_key(key_esc());

    assert!(ed.state.config.picker.is_none());
    assert_eq!(pending_calls(&ed).len(), 1);
    let (proc, args) = pending_calls(&ed)[0];
    assert_eq!(callback_name(proc), "cb");
    assert_eq!(args, &vec![SteelVal::BoolV(false)]);

    // L4 continuation: typing after the close edits the buffer normally.
    ed.feed_key(key('i'));
    ed.feed_key(key('Y'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "Yabc\n");
}

#[test]
fn enter_with_no_match_dismisses_with_false() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_picker_with_callback(&mut ed, &["foo", "bar"], marker("cb"));
    for ch in "zzz".chars() {
        ed.feed_key(key(ch));
    }
    assert_eq!(ed.state.config.picker.as_ref().unwrap().matched_len(), 0);

    ed.feed_key(key_enter());
    assert!(ed.state.config.picker.is_none());
    let (_, args) = pending_calls(&ed)[0];
    assert_eq!(args, &vec![SteelVal::BoolV(false)]);
}

// ── any-mode open, clears a live completion session ─────────────────────────

#[test]
fn open_from_insert_mode_allowed_and_clears_completion() {
    let mut ed = editor_from("-[a]>bc\n");
    ed.feed_key(key('i'));
    assert_eq!(ed.state.mode(), Mode::Insert);

    let bid = ed.focused_buffer_id();
    let items =
        vec![StoredCompletionItem::from_json(&serde_json::json!({"label": "foo"})).unwrap()];
    let session = CompletionSession::begin(&ed.state, bid, items, false).unwrap();
    ed.lsp.completion = Some(session);

    open_test_picker(&mut ed, &["one", "two"]);
    assert!(
        ed.lsp.completion.is_none(),
        "opening a picker must clear a live completion session"
    );

    // Still in Insert mode (picker is chrome, not a mode) — but the picker
    // intercept sits above `handle_insert`, so a printable edits the query.
    assert_eq!(ed.state.mode(), Mode::Insert);
    ed.feed_key(key('o'));
    assert_eq!(ed.state.config.picker.as_ref().unwrap().query(), "o");
    assert_eq!(
        ed.doc().text().to_string(),
        "abc\n",
        "buffer must be untouched"
    );
}

// ── Replacing an open picker ─────────────────────────────────────────────────

#[test]
fn open_over_open_picker_fires_old_callback_with_false() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_picker_with_callback(&mut ed, &["one"], marker("first"));
    open_test_picker_with_callback(&mut ed, &["two"], marker("second"));

    assert_eq!(pending_calls(&ed).len(), 1);
    let (proc, args) = pending_calls(&ed)[0];
    assert_eq!(callback_name(proc), "first");
    assert_eq!(args, &vec![SteelVal::BoolV(false)]);

    ed.feed_key(key_esc());
    assert_eq!(pending_calls(&ed).len(), 2);
    let (proc, args) = pending_calls(&ed)[1];
    assert_eq!(callback_name(proc), "second");
    assert_eq!(args, &vec![SteelVal::BoolV(false)]);
}

// ── Token guard (session_for_token) ─────────────────────────────────────────

#[test]
fn session_for_token_finds_the_open_session_by_matching_token() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_picker(&mut ed, &["a"]);
    let token = ed.state.config.picker.as_ref().unwrap().token();
    assert!(picker::session_for_token(&mut ed.state, token).is_some());
}

#[test]
fn session_for_token_rejects_a_stale_token() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_picker(&mut ed, &["a"]);
    let token = ed.state.config.picker.as_ref().unwrap().token();
    assert!(picker::session_for_token(&mut ed.state, token + 1).is_none());
}

#[test]
fn session_for_token_is_none_with_no_picker_open() {
    let mut ed = editor_from("-[a]>bc\n");
    assert!(picker::session_for_token(&mut ed.state, 1).is_none());
}

#[test]
fn picker_feed_rejects_a_stale_token_and_leaves_items_and_pending_untouched() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::host::{PickerFeedMode, PickerOpts, UiHost};

    let mut ed = editor_from("-[a]>bc\n");
    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    let token = host
        .open_picker(
            vec![],
            SteelVal::Void,
            PickerOpts {
                pending: true,
                ..Default::default()
            },
        )
        .unwrap();

    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    assert!(!host.picker_feed(
        token + 1,
        vec![("x".to_string(), SteelVal::StringV("p".into()))],
        PickerFeedMode::Append,
    ));
    let session = ed.state.config.picker.as_ref().unwrap();
    assert_eq!(
        session.total_len(),
        0,
        "a stale-token feed must not touch the item list"
    );
    assert!(
        session.is_pending(),
        "a rejected feed must not clear pending — the real batch hasn't arrived yet"
    );
}

#[test]
fn picker_feed_replace_mode_rejects_a_stale_token_and_leaves_items_untouched() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::host::{PickerFeedMode, PickerOpts, UiHost};

    let mut ed = editor_from("-[a]>bc\n");
    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    let token = host
        .open_picker(
            vec![("a".to_string(), SteelVal::StringV("a".into()))],
            SteelVal::Void,
            PickerOpts::default(),
        )
        .unwrap();

    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    assert!(!host.picker_feed(
        token + 1,
        vec![("z".to_string(), SteelVal::StringV("z".into()))],
        PickerFeedMode::Replace,
    ));
    assert_eq!(
        ed.state.config.picker.as_ref().unwrap().total_len(),
        1,
        "a stale-token replace must leave the existing items untouched"
    );
}

// ── Intercept ordering ───────────────────────────────────────────────────────

#[test]
fn picker_intercepts_ahead_of_menu() {
    let mut ed = editor_from("-[a]>bc\n");
    ed.state.config.menu = Some(crate::ui::popup::MenuModel {
        items: vec!["m0".into(), "m1".into()],
        selected: 0,
        callback: marker("menu-cb"),
    });
    open_test_picker(&mut ed, &["a", "b"]);

    ed.feed_key(key_down());
    assert_eq!(
        ed.state.config.picker.as_ref().unwrap().selected(),
        1,
        "picker must consume the key"
    );
    assert_eq!(
        ed.state.config.menu.as_ref().unwrap().selected,
        0,
        "menu must not see it"
    );
}

// ── Re-rank resets selection/scroll end-to-end ──────────────────────────────

#[test]
fn typing_rerank_resets_selection_and_scroll_end_to_end() {
    let mut ed = editor_from("-[a]>bc\n");
    let items: Vec<String> = (0..20).map(|i| format!("item{i}")).collect();
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    open_test_picker(&mut ed, &refs);
    frame(&mut ed, 60, 16);

    for _ in 0..10 {
        ed.feed_key(key_down());
    }
    assert_ne!(ed.state.config.picker.as_ref().unwrap().selected(), 0);

    ed.feed_key(key('i'));
    assert_eq!(ed.state.config.picker.as_ref().unwrap().selected(), 0);
    assert_eq!(ed.state.config.picker.as_ref().unwrap().scroll(), 0);
}

// ── View lifecycle ───────────────────────────────────────────────────────────

#[test]
fn close_clears_view_next_frame() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_picker(&mut ed, &["one"]);
    frame(&mut ed, 60, 16);
    assert!(
        ed.state.picker_view.read().unwrap().is_some(),
        "sanity: view populated while open"
    );

    ed.feed_key(key_esc());
    frame(&mut ed, 60, 16);
    assert!(
        ed.state.picker_view.read().unwrap().is_none(),
        "view must clear the frame after close"
    );
}

#[test]
fn shrinking_terminal_self_heals_scroll() {
    let mut ed = editor_from("-[a]>bc\n");
    let items: Vec<String> = (0..50).map(|i| format!("item{i}")).collect();
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    open_test_picker(&mut ed, &refs);

    frame(&mut ed, 80, 30);
    for _ in 0..40 {
        ed.feed_key(key_down());
    }

    // Shrinking between frames must not panic, and the next sync must keep
    // `selected_row` valid against the new, smaller window.
    frame(&mut ed, 30, 12);
    let guard = ed.state.picker_view.read().unwrap();
    let state = guard.as_ref().expect("picker still open");
    let row = state
        .selected_row
        .expect("50 matches for an empty query must always select a row");
    assert!(
        row < state.rows.len(),
        "selected_row must stay inside the new window"
    );
}

// ── Full-frame render snapshots ─────────────────────────────────────────────
//
// Uses `Editor::open` (the real constructor, going through `build_pane`) —
// unlike `editor_from`'s minimal harness, this registers `PickerOverlay` so
// the panel actually paints.

fn open_real_editor() -> Editor {
    Editor::open(None, std::sync::Arc::new(|| {})).unwrap()
}

#[test]
fn snapshot_picker_over_populated_buffer_empty_query() {
    let mut ed = open_real_editor();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    for ch in "hello world".chars() {
        ed.feed_key(key('i'));
        ed.feed_key(key(ch));
        ed.feed_key(key_esc());
    }
    open_test_picker(&mut ed, &["alpha", "beta", "gamma"]);

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 12);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let rect = ratatui::layout::Rect::new(0, 0, 40, 12);
    let snap = render_snapshot::render_to_styled_string(&mut ed, rect);
    insta::assert_snapshot!(snap);
}

#[test]
fn snapshot_picker_after_filtering_query() {
    let mut ed = open_real_editor();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    open_test_picker(&mut ed, &["apple", "banana", "apricot"]);
    ed.feed_key(key('a'));
    ed.feed_key(key('p'));

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 12);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let rect = ratatui::layout::Rect::new(0, 0, 40, 12);
    let snap = render_snapshot::render_to_styled_string(&mut ed, rect);
    insta::assert_snapshot!(snap);
}

#[test]
fn snapshot_picker_scrolled_with_selection_highlight() {
    let mut ed = open_real_editor();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    let items: Vec<String> = (0..30).map(|i| format!("item{i}")).collect();
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    open_test_picker(&mut ed, &refs);

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 12);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    for _ in 0..15 {
        ed.feed_key(key_down());
    }

    ed.sync_viewport_dims(40, 12);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let rect = ratatui::layout::Rect::new(0, 0, 40, 12);
    let snap = render_snapshot::render_to_styled_string(&mut ed, rect);
    insta::assert_snapshot!(snap);
}

#[test]
fn snapshot_picker_no_match_state() {
    let mut ed = open_real_editor();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    open_test_picker(&mut ed, &["foo", "bar"]);
    for ch in "zzz".chars() {
        ed.feed_key(key(ch));
    }

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 12);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let rect = ratatui::layout::Rect::new(0, 0, 40, 12);
    let snap = render_snapshot::render_to_styled_string(&mut ed, rect);
    insta::assert_snapshot!(snap);
}

#[test]
fn snapshot_picker_with_prompt() {
    let mut ed = open_real_editor();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    open_test_picker_with_prompt(&mut ed, &["alpha", "beta", "gamma"], "files: ");
    ed.feed_key(key('a'));

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 12);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let rect = ratatui::layout::Rect::new(0, 0, 40, 12);
    let snap = render_snapshot::render_to_styled_string(&mut ed, rect);
    insta::assert_snapshot!(snap);
}
