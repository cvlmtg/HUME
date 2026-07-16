// Class B bottom drawer: (show-drawer-list!
// items on-select) / (close-drawer!), the Normal/Extend-only key intercept
// in `Editor::handle_key` (`handle_drawer_key`), and the engine chrome band
// (see `hume-engine`'s `pane_area_*` tests for the partition math itself).

use std::path::Path;

use super::*;
use crate::editor::scripting_setup::make_init_host;
use hume_engine::pipeline::RenderContext;
use hume_scripting::ScriptingHost;

fn eval_with_real_host(ed: &mut Editor, host: &mut ScriptingHost, source: &str, tmp: &Path) {
    let init_path = tmp.join("init.scm");
    std::fs::write(&init_path, source).unwrap();
    let mut ih = make_init_host(&mut ed.state, &mut ed.view);
    host.eval_init(&init_path, 10_000, &mut ih, Default::default())
        .expect("eval_init");
}

fn run(ed: &mut Editor, tmp: &Path, source: &str) {
    let mut host = ScriptingHost::new();
    eval_with_real_host(ed, &mut host, source, tmp);
    ed.scripting = Some(host);
}

fn arm_three_items(ed: &mut Editor, tmp: &Path) {
    run(
        ed,
        tmp,
        r#"(define-command! "go" "" (lambda ()
             (show-drawer-list! (list "one.rs:1" "two.rs:2" "three.rs:3")
               (lambda (idx) (log! 'info (to-string idx))))))"#,
    );
    type_cmd(ed, ":go");
}

// ── show-drawer-list! / close-drawer! ─────────────────────────────────────────

#[test]
fn show_drawer_list_populates_model_and_view() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());

    assert!(ed.state.drawer.is_some());
    let guard = ed.state.drawer_view.read().unwrap();
    let view = guard.as_ref().expect("view must be populated on open");
    assert_eq!(view.rows, vec!["one.rs:1", "two.rs:2", "three.rs:3"]);
    assert_eq!(view.selected, 0);
}

#[test]
fn close_drawer_drops_the_callback_without_invoking_it() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::host::UiHost;

    let mut ed = editor_from("-[x]>abcdefgh\n");
    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.show_drawer_list(
        vec!["a".to_string(), "b".to_string()],
        steel::rvals::SteelVal::Void,
    )
    .unwrap();
    assert!(ed.state.drawer.is_some());

    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.close_drawer().unwrap();

    assert!(ed.state.drawer.is_none());
    assert!(ed.state.drawer_view.read().unwrap().is_none());
    assert!(
        ed.state.pending_steel_calls.is_empty(),
        "close_drawer must not queue the callback"
    );
}

// ── Esc: closes + calls back with #f ─────────────────────────────────────────

#[test]
fn esc_calls_back_with_false_and_closes() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());

    ed.feed_key(key_esc());
    ed.drain_pending_steel_calls();

    assert_eq!(ed.state.status_msg.clone().unwrap(), "#false");
    assert!(ed.state.drawer.is_none());
    assert!(ed.state.drawer_view.read().unwrap().is_none());
}

// ── Enter: fires the callback and stays open, repeatedly ─────────────────────

#[test]
fn enter_calls_back_and_the_drawer_stays_open() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());

    ed.feed_key(key_enter());
    ed.drain_pending_steel_calls();
    assert_eq!(ed.state.status_msg.clone().unwrap(), "0");
    assert!(ed.state.drawer.is_some(), "must stay open after Enter");

    // Move selection and fire again — the callback must still be usable
    // (cloned, not consumed by the first Enter).
    ed.feed_key(key_down());
    ed.feed_key(key_enter());
    ed.drain_pending_steel_calls();
    assert_eq!(ed.state.status_msg.clone().unwrap(), "1");
    assert!(ed.state.drawer.is_some());
}

// ── Selection clamps at both ends ─────────────────────────────────────────────

#[test]
fn selection_clamps_at_the_top() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());

    ed.feed_key(key_up());
    ed.feed_key(key_up());
    ed.feed_key(key_enter());
    ed.drain_pending_steel_calls();
    assert_eq!(ed.state.status_msg.clone().unwrap(), "0");
}

#[test]
fn selection_clamps_at_the_bottom() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());

    for _ in 0..5 {
        ed.feed_key(key_down());
    }
    ed.feed_key(key_enter());
    ed.drain_pending_steel_calls();
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "2",
        "clamped, not wrapped"
    );
}

// ── Stray key: neither closes nor invokes, but still executes ────────────────

#[test]
fn stray_key_leaves_the_drawer_open_and_uninvoked_but_still_executes() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());

    let head_before = ed.current_selections().primary().head();
    ed.feed_key(key('l')); // move-right — not one of the drawer's keys
    ed.drain_pending_steel_calls();

    assert!(
        ed.state.drawer.is_some(),
        "stray key must not close the drawer"
    );
    assert!(
        ed.state.status_msg.is_none(),
        "stray key must not invoke the callback"
    );
    assert_ne!(
        ed.current_selections().primary().head(),
        head_before,
        "stray key must still execute its normal effect"
    );
}

#[test]
fn long_list_auto_scrolls_to_keep_selection_visible() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    let items_scm: String = (0..20)
        .map(|i| format!("\"item {i}\""))
        .collect::<Vec<_>>()
        .join(" ");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-command! "go" "" (lambda ()
                 (show-drawer-list! (list {items_scm})
                   (lambda (idx) (log! 'info (to-string idx))))))"#
        ),
    );

    // Populate `last_terminal_area` before any key handling needs it — the
    // scroll clamp reads it to agree with what the engine will next paint.
    let mut ctx = RenderContext::new();
    ed.prepare_frame(40, 10, &mut ctx);
    type_cmd(&mut ed, ":go");

    // capacity = min(20 items + 1, 10 rows / 2 = 5) = 5; visible_rows = 4.
    for _ in 0..6 {
        ed.feed_key(key_down());
    }

    let drawer = ed.state.drawer.as_ref().unwrap();
    assert_eq!(drawer.selected, 6);
    assert!(
        drawer.scroll > 0,
        "scroll must advance — only 4 rows are visible but selection moved to row 6"
    );
    assert!(
        drawer.selected + 1 - drawer.scroll <= 4,
        "selected row must stay inside the visible window: selected={} scroll={}",
        drawer.selected,
        drawer.scroll
    );
}

// ── End-to-end: Enter jumps via goto-location!, drawer stays open ────────────

#[test]
fn enter_jump_lands_via_goto_location_and_drawer_stays_open() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[a]>bc\ndef\nghi\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (show-drawer-list! (list "line 3")
               (lambda (idx) (goto-location! (list (current-buffer) 2 1))))))"#,
    );
    type_cmd(&mut ed, ":go");

    ed.feed_key(key_enter());
    ed.drain_pending_steel_calls();

    let head = ed.current_selections().primary().head();
    let bid = ed.focused_buffer_id();
    let text = ed.state.buffers.get(bid).text();
    let line = text.char_to_line(head);
    let col = head - text.line_to_char(line);
    assert_eq!((line, col), (2, 1), "cursor landed at the jump target");
    assert!(
        ed.state.drawer.is_some(),
        "drawer stays open after the jump"
    );
}

// ── Render snapshot: drawer band under the shrunk pane grid ──────────────────

#[test]
fn drawer_renders_under_the_pane_with_selected_row_highlighted() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = Editor::open(None).unwrap();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    ed.feed_key(key('i'));
    for ch in "hello".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_esc());
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (show-drawer-list! (list "src/a.rs:1: unused import" "src/b.rs:9: TODO")
               (lambda (idx) (void)))))"#,
    );
    type_cmd(&mut ed, ":go");

    let mut ctx = RenderContext::new();
    ed.prepare_frame(40, 10, &mut ctx);

    use ratatui::layout::Rect;
    let rect = Rect::new(0, 0, 40, 10);
    let snap = render_snapshot::render_to_styled_string(&mut ed, rect);
    insta::assert_snapshot!(snap);
}
