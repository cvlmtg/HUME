// Selection menu widget: (show-menu! items
// on-select) / (close-menu!), and the Normal/Extend-only key intercept in
// `Editor::handle_key` (`handle_menu_key`).

use hume_grid::Rect;
use std::path::Path;

use super::*;
use hume_engine::pipeline::RenderContext;

fn arm_three_items(ed: &mut Editor, tmp: &Path) {
    run(
        ed,
        tmp,
        r#"(define-typed-command! "go" "" (lambda ()
             (show-menu! (list "Extract function" "Inline variable" "Rename")
               (lambda (idx) (log! 'info (to-string idx))))))"#,
    );
    type_cmd(ed, ":go");
}

// ── Selection + confirm/cancel ────────────────────────────────────────────────

#[test]
fn select_second_item_calls_back_with_index_1() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());
    assert!(ed.state.input.menu().is_some(), "sanity: menu open");

    ed.feed_key(key('j'));
    ed.feed_key(key_enter());
    ed.settle();

    assert_eq!(ed.state.status_msg.clone().unwrap(), "1");
    assert!(
        ed.state.input.menu().is_none(),
        "menu must close after Enter"
    );
}

#[test]
fn esc_calls_back_with_false() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());

    ed.feed_key(key('j'));
    ed.feed_key(key_esc());
    ed.settle();

    assert_eq!(ed.state.status_msg.clone().unwrap(), "#false");
    assert!(ed.state.input.menu().is_none());
}

#[test]
fn selection_clamps_at_the_top() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());

    // At index 0, 'k' must not go negative.
    ed.feed_key(key('k'));
    ed.feed_key(key('k'));
    ed.feed_key(key_enter());
    ed.settle();
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "0",
        "clamped at the top"
    );
}

#[test]
fn selection_clamps_at_the_bottom() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());

    // 3 items (indices 0..=2) — 5 'j' presses must clamp at 2, not wrap.
    for _ in 0..5 {
        ed.feed_key(key('j'));
    }
    ed.feed_key(key_enter());
    ed.settle();
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "2",
        "clamped at the bottom, not wrapped"
    );
}

#[test]
fn arrow_keys_also_move_the_selection() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());

    ed.feed_key(key_down());
    ed.feed_key(key_enter());
    ed.settle();
    assert_eq!(ed.state.status_msg.clone().unwrap(), "1");
}

// ── Stray key dismisses and still executes ───────────────────────────────────

#[test]
fn stray_key_dismisses_the_menu_and_still_executes() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());

    // 'l' (move-right) is not one of the menu's intercepted keys.
    let head_before = ed.current_selections().primary().head();
    ed.feed_key(key('l'));
    ed.settle();

    assert!(
        ed.state.input.menu().is_none(),
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
    use hume_scripting::host::UiHost;

    let mut ed = editor_from("-[x]>abcdefgh\n");
    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.show_menu(
        vec!["a".to_string(), "b".to_string()],
        steel::rvals::SteelVal::Void,
    )
    .unwrap();
    assert!(ed.state.input.menu().is_some(), "sanity: menu open");

    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.close_menu().unwrap();

    assert!(ed.state.input.menu().is_none());
    assert!(
        ed.state.config.pending_work.is_empty(),
        "close_menu must not queue the callback"
    );
}

/// `show-menu!` from Insert is a benign timing issue (a `codeAction`
/// response landing after the user left Normal), not a plugin bug — it
/// drops silently (`Ok`, a `Trace` log entry) rather than erroring, so it
/// never aborts the `run_call_batch` a real async callback is batched into.
#[test]
fn show_menu_from_insert_drops_silently_as_a_mode_layer_race() {
    use crate::editor::Severity;
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::host::UiHost;

    let mut ed = editor_from("-[x]>abcdefgh\n");
    ed.feed_key(key('i')); // enter Insert mode
    assert_eq!(ed.state.mode(), hume_engine::types::EditorMode::Insert);

    let traces_before = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Trace)
        .count();

    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    let result = host.show_menu(vec!["a".to_string()], steel::rvals::SteelVal::Void);
    assert!(
        result.is_ok(),
        "a mode-layer race must never error — it would abort the whole call batch"
    );
    assert!(
        ed.state.input.menu().is_none(),
        "must not have opened while the mode layer isn't Base"
    );
    assert_eq!(
        ed.state
            .message_log
            .entries()
            .filter(|e| e.severity == Severity::Trace)
            .count(),
        traces_before + 1,
        "must log a Trace entry noting the drop"
    );
}

#[test]
fn show_menu_accepted_in_normal_mode() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::host::UiHost;

    let mut ed = editor_from("-[x]>abcdefgh\n");
    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    let result = host.show_menu(vec!["a".to_string()], steel::rvals::SteelVal::Void);
    assert!(result.is_ok());
    assert!(ed.state.input.menu().is_some());
}

/// Same async-staleness rule as `show_menu_from_insert_…` above, triggered
/// the other way: the mode layer is still `Base`, but a picker landed on
/// top of it while the `codeAction` response was in flight. `show-menu!`
/// must not bury a picker under a menu it never asked for — it drops
/// silently, same as the mode-changed case.
#[test]
fn show_menu_drops_silently_when_a_picker_is_open() {
    use crate::editor::Severity;
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::host::{PickerOpts, UiHost};

    let mut ed = editor_from("-[x]>abcdefgh\n");
    let session = crate::editor::picker::PickerSession::new(
        steel::rvals::SteelVal::BoolV(false),
        PickerOpts::default(),
    );
    crate::editor::picker::open_picker(&mut ed.state, &ed.view, session);

    let traces_before = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Trace)
        .count();

    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    let result = host.show_menu(vec!["a".to_string()], steel::rvals::SteelVal::Void);
    assert!(
        result.is_ok(),
        "a stale async response must never error — it would abort the whole call batch"
    );
    assert!(
        ed.state.input.menu().is_none(),
        "must not have opened above the picker"
    );
    assert_eq!(
        ed.state
            .message_log
            .entries()
            .filter(|e| e.severity == Severity::Trace)
            .count(),
        traces_before + 1,
        "must log a Trace entry noting the drop"
    );
    assert!(ed.state.input.picker().is_some(), "the picker stays open");
}

// ── Render snapshot: highlighted row ──────────────────────────────────────────

#[test]
fn selected_row_renders_with_the_menu_selected_scope() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = crate::testing::build_snapshot_theme();
    ed.feed_key(key('i'));
    for ch in "abcdefgh".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_esc());
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda ()
             (show-menu! (list "Extract function" "Inline variable")
               (lambda (idx) (void)))))"#,
    );
    type_cmd(&mut ed, ":go");

    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 10);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let rect = Rect::new(0, 0, 40, 10);
    let snap = render_snapshot::render_to_styled_string(&mut ed, rect);
    insta::assert_snapshot!(snap);
}
