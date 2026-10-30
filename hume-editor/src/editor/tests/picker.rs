// Fuzzy-picker panel: input handling (`Editor::picker_input`), the open
// and close chokepoints (`picker::open_picker`, `picker::close_picker`),
// selection movement (`picker::move_picker`), and the per-frame write side
// (`sync_picker_view`). Sessions are still constructed directly rather than
// through the `picker!` Steel builtin, with marker values standing in for
// keymap bindings, which these tests only ever see queued; see
// `tests/picker_steel.rs` for end-to-end coverage of the Steel surface itself.

use super::*;
use crate::editor::input_stack::picker;
use crate::editor::input_stack::{PickerItem, PickerSession};
use hume_engine::pipeline::RenderContext;
use hume_grid::Rect;
use hume_scripting::host::{PickerBinding, PickerOpts, PickerStep};
use hume_ui::picker_panel::panel_geometry;
use steel::rvals::SteelVal;

fn marker(name: &str) -> SteelVal {
    SteelVal::StringV(name.into())
}

fn binding(key: KeyEvent, name: &str) -> PickerBinding {
    PickerBinding {
        spec: name.to_string(),
        key,
        binding: hume_scripting::Callback::for_test(marker(name)),
    }
}

fn open_test_session(
    ed: &mut Editor,
    items: &[&str],
    keymap: Vec<PickerBinding>,
    opts: PickerOpts,
) {
    let mut session = PickerSession::new(keymap, opts, Vec::new()).expect("no key is bound twice");
    let picker_items: Vec<PickerItem> = items
        .iter()
        .map(|s| PickerItem {
            display: s.to_string(),
            payload: hume_scripting::Rooted::new(SteelVal::StringV((*s).into())),
        })
        .collect();
    session.push(picker_items);
    picker::open_picker(&mut ed.state, &ed.view, session).expect("no `.` is in progress");
}

fn open_test_picker(ed: &mut Editor, items: &[&str]) {
    open_test_session(ed, items, Vec::new(), PickerOpts::default());
}

fn open_test_picker_with_on_close(ed: &mut Editor, items: &[&str], on_close: &str) {
    open_test_session(
        ed,
        items,
        Vec::new(),
        PickerOpts {
            on_close: Some(hume_scripting::Callback::for_test(marker(on_close))),
            ..Default::default()
        },
    );
}

fn open_test_picker_with_prompt(ed: &mut Editor, items: &[&str], prompt: &str) {
    open_test_session(
        ed,
        items,
        Vec::new(),
        PickerOpts {
            prompt: prompt.to_string(),
            ..Default::default()
        },
    );
}

/// `picker-move!` on the open picker.
fn move_by(ed: &mut Editor, count: isize, step: PickerStep) {
    let token = ed.state.input.picker().expect("a picker is open").token();
    picker::move_picker(&mut ed.state, &ed.view, token, count, step);
}

fn selected(ed: &Editor) -> usize {
    ed.state
        .input
        .picker()
        .expect("a picker is open")
        .selected()
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

    let picker = ed.state.input.picker().expect("picker still open");
    assert_eq!(picker.query(), "z");
    assert_eq!(
        ed.doc().text().to_string(),
        "abc\n",
        "buffer must be untouched"
    );
}

// ── Selection movement ──────────────────────────────────────────────────────

#[test]
fn moving_by_rows_steps_one_item_per_count() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_picker(&mut ed, &["a", "b", "c"]);
    frame(&mut ed, 60, 16);

    move_by(&mut ed, 1, PickerStep::Row);
    assert_eq!(selected(&ed), 1);
    move_by(&mut ed, 1, PickerStep::Row);
    assert_eq!(selected(&ed), 2);
    move_by(&mut ed, -2, PickerStep::Row);
    assert_eq!(selected(&ed), 0);
}

#[test]
fn moving_by_rows_stops_at_both_ends_without_wrapping() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_picker(&mut ed, &["a", "b", "c"]);
    frame(&mut ed, 60, 16);

    move_by(&mut ed, -1, PickerStep::Row);
    assert_eq!(selected(&ed), 0);
    move_by(&mut ed, 10, PickerStep::Row);
    assert_eq!(selected(&ed), 2);
}

#[test]
fn moving_by_pages_steps_the_panel_list_rows() {
    let mut ed = editor_from("-[a]>bc\n");
    let items: Vec<String> = (0..50).map(|i| format!("item{i}")).collect();
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    open_test_picker(&mut ed, &refs);
    frame(&mut ed, 60, 16);

    let geo = panel_geometry(ed.screen_layout().pane_area()).expect("viable geometry at 60x16");
    move_by(&mut ed, 1, PickerStep::Page);
    assert_eq!(selected(&ed), geo.list_rows);
    move_by(&mut ed, -1, PickerStep::Page);
    assert_eq!(selected(&ed), 0);
}

/// The panel's list rows on the headless 80x25 terminal: its 80x24 pane
/// area, below the statusline.
fn headless_list_rows() -> usize {
    panel_geometry(Rect::new(0, 0, 80, 24))
        .expect("viable geometry at 80x24")
        .list_rows
}

#[test]
fn moving_by_pages_before_the_first_frame_pages_against_the_headless_terminal() {
    let mut ed = editor_from("-[a]>bc\n");
    let items: Vec<String> = (0..50).map(|i| format!("item{i}")).collect();
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    open_test_picker(&mut ed, &refs);

    move_by(&mut ed, 1, PickerStep::Page);
    assert_eq!(selected(&ed), headless_list_rows());
    move_by(&mut ed, -1, PickerStep::Page);
    assert_eq!(selected(&ed), 0);
}

#[test]
fn moving_by_pages_on_a_terminal_too_small_for_the_panel_is_a_safe_noop() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_picker(&mut ed, &["a", "b", "c"]);
    frame(&mut ed, 10, 4);

    move_by(&mut ed, 1, PickerStep::Page);
    assert_eq!(selected(&ed), 0);
    move_by(&mut ed, 1, PickerStep::HalfPage);
    assert_eq!(selected(&ed), 0);
}

#[test]
fn moving_by_half_pages_steps_half_the_list_rows() {
    let mut ed = editor_from("-[a]>bc\n");
    let items: Vec<String> = (0..50).map(|i| format!("item{i}")).collect();
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    open_test_picker(&mut ed, &refs);
    frame(&mut ed, 60, 16);

    let geo = panel_geometry(ed.screen_layout().pane_area()).expect("viable geometry at 60x16");
    move_by(&mut ed, 1, PickerStep::HalfPage);
    assert_eq!(selected(&ed), geo.list_rows / 2);
    move_by(&mut ed, -1, PickerStep::HalfPage);
    assert_eq!(selected(&ed), 0);
}

#[test]
fn moving_by_half_pages_before_the_first_frame_pages_against_the_headless_terminal() {
    let mut ed = editor_from("-[a]>bc\n");
    let items: Vec<String> = (0..50).map(|i| format!("item{i}")).collect();
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    open_test_picker(&mut ed, &refs);

    move_by(&mut ed, 1, PickerStep::HalfPage);
    assert_eq!(selected(&ed), headless_list_rows() / 2);
    move_by(&mut ed, -1, PickerStep::HalfPage);
    assert_eq!(selected(&ed), 0);
}

#[test]
fn moving_a_picker_through_a_stale_token_is_a_noop() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_picker(&mut ed, &["a", "b", "c"]);

    picker::move_picker(
        &mut ed.state,
        &ed.view,
        hume_core::HostToken::mint(),
        1,
        PickerStep::Row,
    );
    assert_eq!(selected(&ed), 0);
}

// ── Backspace ────────────────────────────────────────────────────────────────

#[test]
fn backspace_pops_full_grapheme() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_picker(&mut ed, &["one"]);
    ed.feed_key(key('o'));
    ed.feed_key(key('n'));
    ed.feed_key(key_backspace());
    assert_eq!(ed.state.input.picker().unwrap().query(), "o");
}

#[test]
fn backspace_on_empty_keeps_picker_open() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_picker(&mut ed, &["one"]);
    ed.feed_key(key_backspace());
    assert!(
        ed.state.input.picker().is_some(),
        "backspace on an empty query must not close the picker"
    );
    assert_eq!(ed.state.input.picker().unwrap().query(), "");
}

// ── Keymap ───────────────────────────────────────────────────────────────────

#[test]
fn bound_key_queues_its_binding_with_the_token_and_selected_payload() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_session(
        &mut ed,
        &["one", "two"],
        vec![binding(key_ctrl('t'), "ctrl-t")],
        PickerOpts::default(),
    );
    let token = ed.state.input.picker().unwrap().token();

    ed.feed_key(key_ctrl('t'));

    assert!(
        ed.state.input.picker().is_some(),
        "a binding closes the picker only by calling picker-close!"
    );
    assert_eq!(pending_calls(&ed).len(), 1);
    let (proc, args) = pending_calls(&ed).remove(0);
    assert_eq!(callback_name(&proc), "ctrl-t");
    assert_eq!(
        args,
        vec![hume_scripting::host::token_to_steel(token), marker("one")],
        "the token, then the top-ranked item's payload"
    );
}

#[test]
fn bound_key_with_nothing_matching_hands_its_binding_false() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_session(
        &mut ed,
        &["foo", "bar"],
        vec![binding(key_enter(), "enter")],
        PickerOpts::default(),
    );
    for ch in "zzz".chars() {
        ed.feed_key(key(ch));
    }
    assert_eq!(ed.state.input.picker().unwrap().matched_len(), 0);

    ed.feed_key(key_enter());
    let (_, args) = pending_calls(&ed).remove(0);
    assert_eq!(args[1], SteelVal::BoolV(false));
}

#[test]
fn bound_plain_character_runs_its_binding_instead_of_typing() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_session(
        &mut ed,
        &["one"],
        vec![binding(key('j'), "j")],
        PickerOpts::default(),
    );

    ed.feed_key(key('j'));

    assert_eq!(ed.state.input.picker().unwrap().query(), "");
    assert_eq!(callback_name(&pending_calls(&ed)[0].0), "j");
}

#[test]
fn bound_backspace_runs_its_binding_instead_of_deleting() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_session(
        &mut ed,
        &["one"],
        vec![binding(key_backspace(), "backspace")],
        PickerOpts::default(),
    );
    ed.feed_key(key('o'));

    ed.feed_key(key_backspace());

    assert_eq!(ed.state.input.picker().unwrap().query(), "o");
    assert_eq!(callback_name(&pending_calls(&ed)[0].0), "backspace");
}

#[test]
fn a_binding_reaches_its_key_however_the_terminal_reports_case() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_session(
        &mut ed,
        &["one"],
        vec![binding(
            KeyEvent::new(KeyCode::Char('j'), Modifiers::SHIFT),
            "J",
        )],
        PickerOpts::default(),
    );

    ed.feed_key(key('J'));

    assert_eq!(ed.state.input.picker().unwrap().query(), "");
    assert_eq!(callback_name(&pending_calls(&ed)[0].0), "J");
}

#[test]
fn two_specs_naming_one_key_refuse_to_open() {
    let keymap = vec![
        PickerBinding {
            spec: "ctrl-n".to_string(),
            key: key_ctrl('n'),
            binding: hume_scripting::Callback::for_test(marker("a")),
        },
        PickerBinding {
            spec: "c-n".to_string(),
            key: key_ctrl('n'),
            binding: hume_scripting::Callback::for_test(marker("b")),
        },
    ];
    let Err(err) = PickerSession::new(keymap, PickerOpts::default(), Vec::new()) else {
        panic!("a keymap naming one key twice must not open a picker");
    };
    assert!(
        err.contains("\"ctrl-n\"") && err.contains("\"c-n\""),
        "the error names both specs, got {err:?}"
    );
}

// ── Unbound keys ─────────────────────────────────────────────────────────────

#[test]
fn unbound_keys_are_consumed_and_ignored() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_picker(&mut ed, &["one", "two"]);
    for stray in [
        key_enter(),
        key_esc(),
        key_down(),
        key_ctrl('n'),
        key_ctrl('t'),
        key_left(),
        key_tab(),
        KeyEvent::new(KeyCode::Home, Modifiers::NONE),
    ] {
        ed.feed_key(stray);
    }
    let picker = ed
        .state
        .input
        .picker()
        .expect("unbound keys never close the picker");
    assert_eq!(picker.query(), "");
    assert_eq!(picker.selected(), 0);
    assert_eq!(pending_calls(&ed).len(), 0);
    assert_eq!(
        ed.doc().text().to_string(),
        "abc\n",
        "buffer must be untouched"
    );
}

// ── Closing fires #:on-close ────────────────────────────────────────────────

#[test]
fn close_fires_on_close_once_and_typing_resumes_editing() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_picker_with_on_close(&mut ed, &["one"], "closed");

    picker::close_picker(&mut ed.state, &ed.view);

    assert!(ed.state.input.picker().is_none());
    assert_eq!(pending_calls(&ed).len(), 1);
    let (proc, args) = pending_calls(&ed).remove(0);
    assert_eq!(callback_name(&proc), "closed");
    assert!(args.is_empty());

    picker::close_picker(&mut ed.state, &ed.view);
    assert_eq!(
        pending_calls(&ed).len(),
        1,
        "closing an already-closed picker fires nothing"
    );

    ed.feed_key(key('i'));
    ed.feed_key(key('Y'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "Yabc\n");
}

#[test]
fn close_without_on_close_queues_nothing() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_picker(&mut ed, &["one"]);

    picker::close_picker(&mut ed.state, &ed.view);

    assert!(ed.state.input.picker().is_none());
    assert_eq!(pending_calls(&ed).len(), 0);
}

// ── any-mode open, clears a live completion session ─────────────────────────

#[test]
fn open_from_insert_mode_allowed_and_clears_completion() {
    let mut ed = editor_from("-[a]>bc\n");
    ed.feed_key(key('i'));
    assert_eq!(ed.state.mode(), Mode::Insert);

    open_completion_session(&mut ed, &["foo"]);

    open_test_picker(&mut ed, &["one", "two"]);
    assert!(
        ed.state.input.buffer_completion().is_none(),
        "opening a picker must clear a live completion session"
    );

    // Still in Insert mode (picker is chrome, not a mode), but the `Picker`
    // layer sits above `Insert` on the stack, so a printable edits the query.
    assert_eq!(ed.state.mode(), Mode::Insert);
    ed.feed_key(key('o'));
    assert_eq!(ed.state.input.picker().unwrap().query(), "o");
    assert_eq!(
        ed.doc().text().to_string(),
        "abc\n",
        "buffer must be untouched"
    );
}

/// `close_picker` closing a picker with an `Insert` session stacked above it
/// (a Steel timer entering Insert while the picker owns the keyboard;
/// there's no key path for this, since the picker is full-modal) must run
/// `Insert`'s own teardown on the way out (commit the edit group, clear
/// `active_session`) via `EditorState::take_layer`, and must still fire the
/// picker's own `#:on-close` exactly once: not twice, and not silently
/// dropped by `Insert`'s teardown running first.
///
/// Acting only on the last removed layer (the picker) would skip `Insert`'s
/// `tear_down`, leaving `active_session` set and the edit group open.
#[test]
fn close_picker_tears_down_an_insert_session_stacked_above_it() {
    use crate::editor::commands::{FocusedPane, cmd_insert_at_selection_start};
    use hume_ops::MotionMode;

    let mut ed = editor_from("-[a]>bc\n");
    open_test_picker_with_on_close(&mut ed, &["one"], "closed");
    assert!(ed.state.input.picker().is_some(), "sanity: picker open");

    // Lands above the picker the same ungated way a timer would.
    let fp = FocusedPane::current(&ed.view);
    cmd_insert_at_selection_start(
        &mut ed.state,
        &mut ed.view,
        fp,
        Count::typed(1),
        MotionMode::Move,
    )
    .unwrap();
    assert_eq!(
        ed.state.mode(),
        Mode::Insert,
        "sanity: Insert above the picker"
    );
    assert!(ed.state.open_session().is_some(), "sanity: session open");
    // Insert is `top()` now, so this reaches its own key handler, same as
    // ordinary typing, not the picker's query.
    ed.handle_key(key('X'));
    assert_eq!(ed.doc().text().to_string(), "Xabc\n");

    crate::editor::input_stack::picker::close_picker(&mut ed.state, &ed.view);

    assert!(ed.state.input.picker().is_none(), "the picker must be gone");
    assert!(
        ed.state.open_session().is_none(),
        "Insert's own teardown must have run, clearing the session"
    );
    assert_eq!(
        pending_calls(&ed).len(),
        1,
        "#:on-close must fire exactly once, not once per removed layer"
    );
    assert_eq!(callback_name(&pending_calls(&ed)[0].0), "closed");
}

// ── Replacing an open picker ─────────────────────────────────────────────────

#[test]
fn open_over_open_picker_fires_the_old_on_close() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_picker_with_on_close(&mut ed, &["one"], "first");
    open_test_picker_with_on_close(&mut ed, &["two"], "second");

    assert_eq!(pending_calls(&ed).len(), 1);
    assert_eq!(callback_name(&pending_calls(&ed)[0].0), "first");

    picker::close_picker(&mut ed.state, &ed.view);
    assert_eq!(pending_calls(&ed).len(), 2);
    assert_eq!(callback_name(&pending_calls(&ed)[1].0), "second");
}

// ── Token guard (session_for_token) ─────────────────────────────────────────

#[test]
fn session_for_token_finds_the_open_session_by_matching_token() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_picker(&mut ed, &["a"]);
    let token = ed.state.input.picker().unwrap().token();
    assert!(picker::session_for_token(&mut ed.state, token).is_some());
}

#[test]
fn session_for_token_rejects_a_stale_token() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_picker(&mut ed, &["a"]);
    assert!(picker::session_for_token(&mut ed.state, hume_core::HostToken::mint()).is_none());
}

#[test]
fn session_for_token_is_none_with_no_picker_open() {
    let mut ed = editor_from("-[a]>bc\n");
    assert!(picker::session_for_token(&mut ed.state, hume_core::HostToken::mint()).is_none());
}

#[test]
fn picker_feed_rejects_a_stale_token_and_leaves_items_and_pending_untouched() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::host::{PickerFeedMode, PickerOpts, UiHost};

    let mut ed = editor_from("-[a]>bc\n");
    let pane = focused_pane(&ed);
    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.open_picker(
        hume_scripting::host::FocusedHandle::for_test(pane),
        vec![],
        Vec::new(),
        PickerOpts {
            pending: true,
            ..Default::default()
        },
    )
    .unwrap();

    let _pane = focused_pane(&ed);
    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    assert!(!host.picker_feed(
        hume_core::HostToken::mint(),
        vec![(
            "x".to_string(),
            hume_scripting::Rooted::new(SteelVal::StringV("p".into()))
        )],
        PickerFeedMode::Append,
    ));
    let session = ed.state.input.picker().unwrap();
    assert_eq!(
        session.total_len(),
        0,
        "a stale-token feed must not touch the item list"
    );
    assert!(
        session.is_pending(),
        "a rejected feed must not clear pending: the real batch hasn't arrived yet"
    );
}

#[test]
fn picker_feed_replace_mode_rejects_a_stale_token_and_leaves_items_untouched() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::host::{PickerFeedMode, PickerOpts, UiHost};

    let mut ed = editor_from("-[a]>bc\n");
    let pane = focused_pane(&ed);
    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.open_picker(
        hume_scripting::host::FocusedHandle::for_test(pane),
        vec![(
            "a".to_string(),
            hume_scripting::Rooted::new(SteelVal::StringV("a".into())),
        )],
        Vec::new(),
        PickerOpts::default(),
    )
    .unwrap();

    let _pane = focused_pane(&ed);
    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    assert!(!host.picker_feed(
        hume_core::HostToken::mint(),
        vec![(
            "z".to_string(),
            hume_scripting::Rooted::new(SteelVal::StringV("z".into()))
        )],
        PickerFeedMode::Replace,
    ));
    assert_eq!(
        ed.state.input.picker().unwrap().total_len(),
        1,
        "a stale-token replace must leave the existing items untouched"
    );
}

// ── Overlay stacking ──────────────────────────────────────────────────────────

/// A picker opening while a menu is already up lands above it, not over a
/// refusal: `picker!` is key-triggered like every synchronous opener, so
/// dispatch order already proves the stack is exactly where the key path
/// left it. The menu survives underneath, swallowed by the picker's own
/// full-modal input policy, and resumes driving its own keys once the
/// picker retires.
#[test]
fn picker_opens_over_a_live_menu_and_the_menu_resumes_once_it_closes() {
    let mut ed = editor_from("-[a]>bc\n");
    ed.state.push_layer(
        &ed.view,
        crate::editor::input_stack::MenuLayer::for_test(
            crate::editor::tests::script_rows(&["m0", "m1"]),
            hume_scripting::Callback::for_test(marker("menu-cb")),
        ),
    );

    let mut session = PickerSession::new(Vec::new(), PickerOpts::default(), Vec::new())
        .expect("an empty keymap repeats no key");
    session.push(vec![PickerItem {
        display: "a".to_string(),
        payload: hume_scripting::Rooted::new(SteelVal::StringV("a".into())),
    }]);
    picker::open_picker(&mut ed.state, &ed.view, session).expect("no `.` is in progress");

    assert!(ed.state.input.picker().is_some(), "the picker opened");
    assert!(
        ed.state.input.menu().is_some(),
        "the menu survives beneath it"
    );
    assert!(
        ed.state
            .input
            .is::<crate::editor::input_stack::PickerLayer>(ed.state.input.top()),
        "the picker, not the menu, drives input while it's open"
    );

    picker::close_picker(&mut ed.state, &ed.view);

    assert!(ed.state.input.picker().is_none(), "the picker closed");
    assert!(ed.state.input.menu().is_some(), "the menu is still there");
    ed.feed_key(key('j'));
    assert_eq!(
        ed.state.input.menu().unwrap().selected,
        1,
        "the menu must drive its own keys again once the picker is gone"
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

    move_by(&mut ed, 10, PickerStep::Row);
    assert_ne!(selected(&ed), 0);

    ed.feed_key(key('i'));
    assert_eq!(ed.state.input.picker().unwrap().selected(), 0);
    assert_eq!(ed.state.input.picker().unwrap().scroll(), 0);
}

// ── View lifecycle ───────────────────────────────────────────────────────────

#[test]
fn close_clears_view_next_frame() {
    let mut ed = editor_from("-[a]>bc\n");
    open_test_picker(&mut ed, &["one"]);
    frame(&mut ed, 60, 16);
    assert!(
        ed.state.views.picker.read().is_some(),
        "sanity: view populated while open"
    );

    picker::close_picker(&mut ed.state, &ed.view);
    frame(&mut ed, 60, 16);
    assert!(
        ed.state.views.picker.read().is_none(),
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
    move_by(&mut ed, 40, PickerStep::Row);

    // Shrinking between frames must not panic, and the next sync must keep
    // `selected_row` valid against the new, smaller window.
    frame(&mut ed, 30, 12);
    let guard = ed.state.views.picker.read();
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
// Uses `Editor::open` (the real constructor, going through `build_pane`):
// unlike `editor_from`'s minimal harness, this registers `PickerOverlay` so
// the panel actually paints.

fn open_real_editor() -> Editor {
    open_headless(None)
}

#[test]
fn snapshot_picker_over_populated_buffer_empty_query() {
    let mut ed = open_real_editor();
    ed.view.theme = crate::testing::build_snapshot_theme();
    // One session, not one `i`/char/`Esc` per character: with
    // `insert-select-on-exit` on, each `Esc` selects the char just typed, so a
    // following `i` would collapse back to its start (not past it) and type
    // the string in reverse.
    ed.feed_key(key('i'));
    for ch in "hello world".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_esc());
    open_test_picker(&mut ed, &["alpha", "beta", "gamma"]);

    let mut ctx = RenderContext::new();
    ed.resize_terminal(40, 12);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let rect = Rect::new(0, 0, 40, 12);
    let snap = render_snapshot::render_to_styled_string(&mut ed, rect);
    insta::assert_snapshot!(snap);
}

#[test]
fn snapshot_picker_after_filtering_query() {
    let mut ed = open_real_editor();
    ed.view.theme = crate::testing::build_snapshot_theme();
    open_test_picker(&mut ed, &["apple", "banana", "apricot"]);
    ed.feed_key(key('a'));
    ed.feed_key(key('p'));

    let mut ctx = RenderContext::new();
    ed.resize_terminal(40, 12);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let rect = Rect::new(0, 0, 40, 12);
    let snap = render_snapshot::render_to_styled_string(&mut ed, rect);
    insta::assert_snapshot!(snap);
}

#[test]
fn snapshot_picker_scrolled_with_selection_highlight() {
    let mut ed = open_real_editor();
    ed.view.theme = crate::testing::build_snapshot_theme();
    let items: Vec<String> = (0..30).map(|i| format!("item{i}")).collect();
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    open_test_picker(&mut ed, &refs);

    let mut ctx = RenderContext::new();
    ed.resize_terminal(40, 12);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    move_by(&mut ed, 15, PickerStep::Row);

    ed.resize_terminal(40, 12);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let rect = Rect::new(0, 0, 40, 12);
    let snap = render_snapshot::render_to_styled_string(&mut ed, rect);
    insta::assert_snapshot!(snap);
}

#[test]
fn snapshot_picker_no_match_state() {
    let mut ed = open_real_editor();
    ed.view.theme = crate::testing::build_snapshot_theme();
    open_test_picker(&mut ed, &["foo", "bar"]);
    for ch in "zzz".chars() {
        ed.feed_key(key(ch));
    }

    let mut ctx = RenderContext::new();
    ed.resize_terminal(40, 12);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let rect = Rect::new(0, 0, 40, 12);
    let snap = render_snapshot::render_to_styled_string(&mut ed, rect);
    insta::assert_snapshot!(snap);
}

#[test]
fn snapshot_picker_with_prompt() {
    let mut ed = open_real_editor();
    ed.view.theme = crate::testing::build_snapshot_theme();
    open_test_picker_with_prompt(&mut ed, &["alpha", "beta", "gamma"], "files: ");
    ed.feed_key(key('a'));

    let mut ctx = RenderContext::new();
    ed.resize_terminal(40, 12);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let rect = Rect::new(0, 0, 40, 12);
    let snap = render_snapshot::render_to_styled_string(&mut ed, rect);
    insta::assert_snapshot!(snap);
}
