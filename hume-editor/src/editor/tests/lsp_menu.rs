// Selection menu widget: (show-menu! items
// on-select) / (close-menu!), and the Normal/Extend-only key intercept in
// `Editor::handle_key` (`handle_menu_key`).

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
             (show-menu! (list "Extract function" "Inline variable" "Rename")
               (lambda (idx) (log! 'info (to-string idx))))))"#,
    );
    type_cmd(ed, ":go");
}

// ── Selection + confirm/cancel ────────────────────────────────────────────────

#[test]
fn select_second_item_calls_back_with_index_1() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());
    assert!(ed.state.menu.is_some(), "sanity: menu open");

    ed.feed_key(key('j'));
    ed.feed_key(key_enter());
    ed.drain_pending_steel_calls();

    assert_eq!(ed.state.status_msg.clone().unwrap(), "1");
    assert!(ed.state.menu.is_none(), "menu must close after Enter");
}

#[test]
fn esc_calls_back_with_false() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());

    ed.feed_key(key('j'));
    ed.feed_key(key_esc());
    ed.drain_pending_steel_calls();

    assert_eq!(ed.state.status_msg.clone().unwrap(), "#false");
    assert!(ed.state.menu.is_none());
}

#[test]
fn selection_clamps_at_the_top() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());

    // At index 0, 'k' must not go negative.
    ed.feed_key(key('k'));
    ed.feed_key(key('k'));
    ed.feed_key(key_enter());
    ed.drain_pending_steel_calls();
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "0",
        "clamped at the top"
    );
}

#[test]
fn selection_clamps_at_the_bottom() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());

    // 3 items (indices 0..=2) — 5 'j' presses must clamp at 2, not wrap.
    for _ in 0..5 {
        ed.feed_key(key('j'));
    }
    ed.feed_key(key_enter());
    ed.drain_pending_steel_calls();
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "2",
        "clamped at the bottom, not wrapped"
    );
}

#[test]
fn arrow_keys_also_move_the_selection() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());

    ed.feed_key(key_down());
    ed.feed_key(key_enter());
    ed.drain_pending_steel_calls();
    assert_eq!(ed.state.status_msg.clone().unwrap(), "1");
}

// ── Stray key dismisses and still executes ───────────────────────────────────

#[test]
fn stray_key_dismisses_the_menu_and_still_executes() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());

    // 'l' (move-right) is not one of the menu's intercepted keys.
    let head_before = ed.current_selections().primary().head();
    ed.feed_key(key('l'));
    ed.drain_pending_steel_calls();

    assert!(
        ed.state.menu.is_none(),
        "the stray key must dismiss the menu"
    );
    assert_eq!(ed.state.status_msg.clone().unwrap(), "#false");
    assert_ne!(
        ed.current_selections().primary().head(),
        head_before,
        "the stray key must still execute its normal effect"
    );
}

// ── close-menu! ────────────────────────────────────────────────────────────────

#[test]
fn close_menu_drops_the_callback_without_invoking_it() {
    // Driven directly through the host, not `type_cmd`: typing the `:` to
    // invoke a `:close`-style command is itself a "stray key" that would
    // dismiss the menu (with `#f`) before the command even runs — this
    // test wants to isolate `close_menu`'s own behavior from that intercept.
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::host::EditorHost;

    let mut ed = editor_from("-[x]>abcdefgh\n");
    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.show_menu(
        vec!["a".to_string(), "b".to_string()],
        steel::rvals::SteelVal::Void,
    )
    .unwrap();
    assert!(ed.state.menu.is_some(), "sanity: menu open");

    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.close_menu().unwrap();

    assert!(ed.state.menu.is_none());
    assert!(
        ed.state.pending_steel_calls.is_empty(),
        "close_menu must not queue the callback"
    );
}

#[test]
fn show_menu_rejected_outside_normal_extend_mode() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::host::EditorHost;

    let mut ed = editor_from("-[x]>abcdefgh\n");
    ed.feed_key(key('i')); // enter Insert mode
    assert_eq!(ed.state.mode(), hume_engine::types::EditorMode::Insert);

    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    let result = host.show_menu(vec!["a".to_string()], steel::rvals::SteelVal::Void);
    assert!(
        result.is_err(),
        "show-menu! must reject Insert mode — a menu that can't be driven is worse than none"
    );
    assert!(
        ed.state.menu.is_none(),
        "must not have opened despite the error"
    );
}

#[test]
fn show_menu_accepted_in_normal_mode() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::host::EditorHost;

    let mut ed = editor_from("-[x]>abcdefgh\n");
    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    let result = host.show_menu(vec!["a".to_string()], steel::rvals::SteelVal::Void);
    assert!(result.is_ok());
    assert!(ed.state.menu.is_some());
}

// ── Render snapshot: highlighted row ──────────────────────────────────────────

#[test]
fn selected_row_renders_with_the_menu_selected_scope() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = Editor::open(None).unwrap();
    ed.view.theme = crate::ui::theme::build_dark_theme_for_snapshot_tests();
    ed.feed_key(key('i'));
    for ch in "abcdefgh".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_esc());
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "go" "" (lambda ()
             (show-menu! (list "Extract function" "Inline variable")
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
