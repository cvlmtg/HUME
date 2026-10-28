// Selection menu widget: (show-menu! items
// on-select) / (close-menu! token), and the Normal/Extend-only input handling in
// `Editor::menu_input`.

use hume_grid::Rect;
use std::path::Path;

use super::*;
use hume_engine::pipeline::RenderContext;

fn arm_three_items(ed: &mut Editor, tmp: &Path) {
    run(
        ed,
        tmp,
        r#"(define-typed-command! "go" "" (lambda (pane)
             (show-menu! pane (list (cons "Extract function" 'extract)
                                    (cons "Inline variable" 'inline)
                                    (cons "Rename" 'rename))
               (lambda (choice) (log! 'info (to-string choice))))))"#,
    );
    type_cmd(ed, ":go");
}

#[test]
fn show_menu_refuses_a_false_payload_and_opens_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda (pane)
             (show-menu! pane (list (cons "No payload" #f)) (lambda (choice) (void)))))"#,
    );
    type_cmd(&mut ed, ":go");
    ed.settle();

    assert!(
        has_error(&ed, "item payload must not be #f"),
        "errors: {:?}",
        error_log(&ed)
    );
    assert!(ed.state.input.menu().is_none());
}

#[test]
fn show_menu_with_empty_items_raises_and_opens_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda (pane)
             (show-menu! pane (list) (lambda (choice) (void)))))"#,
    );
    type_cmd(&mut ed, ":go");
    ed.settle();

    assert!(
        has_error(&ed, "show-menu!: items: must not be empty"),
        "errors: {:?}",
        error_log(&ed)
    );
    assert!(ed.state.input.menu().is_none());
}

// ── Selection + confirm/cancel ────────────────────────────────────────────────

#[test]
fn select_second_item_calls_back_with_its_payload() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());
    assert!(ed.state.input.menu().is_some(), "sanity: menu open");

    ed.feed_key(key('j'));
    ed.feed_key(key_enter());
    ed.settle();

    assert_eq!(ed.state.status_msg.clone().unwrap(), "inline");
    assert!(
        ed.state.input.menu().is_none(),
        "menu must close after Enter"
    );
}

#[test]
fn esc_calls_back_with_false() {
    let tmp = tempfile::tempdir().unwrap();
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
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());

    // At index 0, 'k' must not go negative.
    ed.feed_key(key('k'));
    ed.feed_key(key('k'));
    ed.feed_key(key_enter());
    ed.settle();
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "extract",
        "clamped at the top"
    );
}

#[test]
fn selection_clamps_at_the_bottom() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());

    // 3 items (indices 0..=2): 5 'j' presses must clamp at 2, not wrap.
    for _ in 0..5 {
        ed.feed_key(key('j'));
    }
    ed.feed_key(key_enter());
    ed.settle();
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "rename",
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
    ed.settle();
    assert_eq!(ed.state.status_msg.clone().unwrap(), "inline");
}

// ── Stray key dismisses and still executes ───────────────────────────────────

#[test]
fn stray_key_dismisses_the_menu_and_still_executes() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());

    // 'l' (move-right) is not one of the keys `menu_input` handles.
    let head_before = ed.current_view().primary().head().offset();
    ed.feed_key(key('l'));
    ed.settle();

    assert!(
        ed.state.input.menu().is_none(),
        "the stray key must dismiss the menu"
    );
    assert_eq!(ed.state.status_msg.clone().unwrap(), "#false");
    assert_ne!(
        ed.current_view().primary().head().offset(),
        head_before,
        "the stray key must still execute its normal effect"
    );
}

// ── close-menu! ────────────────────────────────────────────────────────────────

#[test]
fn close_menu_drops_the_callback_without_invoking_it() {
    // Driven directly through the host, not `type_cmd`: typing the `:` to
    // invoke a `:close`-style command is itself a "stray key" that would
    // dismiss the menu (with `#f`) before the command even runs. This
    // test wants to isolate `close_menu`'s own behavior from `menu_input`'s
    // stray-key handling.
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::host::UiHost;

    let mut ed = editor_from("-[x]>abcdefgh\n");
    let pane = focused_pane(&ed);
    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    let token = host
        .show_menu(
            pane,
            script_rows(&["a", "b"]),
            hume_scripting::Callback::for_test(steel::rvals::SteelVal::Void),
        )
        .unwrap()
        .expect("a menu opened from Normal is not dropped");
    assert!(ed.state.input.menu().is_some(), "sanity: menu open");

    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.close_menu(hume_core::HostToken::mint()).unwrap();
    assert!(
        ed.state.input.menu().is_some(),
        "a token the open menu doesn't carry must leave it open"
    );

    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.close_menu(token).unwrap();

    assert!(ed.state.input.menu().is_none());
    assert!(
        ed.state.config.pending_work.is_empty(),
        "close_menu must not queue the callback"
    );
}

/// `show-menu!` from Insert never errors (it would abort the whole
/// `run_call_batch` a real async callback is batched into) but is not a
/// silent no-op either: reaching this from the wrong mode is something
/// worth the user's attention, whether it's a genuine timing issue (a
/// `codeAction` response landing after the user left Normal) or a plugin
/// calling it from a mode that was never going to accept it, so it logs a
/// `Warning`, naming the mode that refused it.
#[test]
fn show_menu_from_insert_warns_instead_of_opening() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_core::Severity;
    use hume_scripting::host::UiHost;

    let mut ed = editor_from("-[x]>abcdefgh\n");
    ed.feed_key(key('i')); // enter Insert mode
    assert_eq!(ed.state.mode(), hume_engine::types::EditorMode::Insert);

    let warnings_before = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Warning)
        .count();

    let pane = focused_pane(&ed);
    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    let result = host.show_menu(
        pane,
        script_rows(&["a"]),
        hume_scripting::Callback::for_test(steel::rvals::SteelVal::Void),
    );
    assert!(
        result.is_ok(),
        "a mode-layer race must never error: it would abort the whole call batch"
    );
    assert!(
        ed.state.input.menu().is_none(),
        "must not have opened while the mode layer isn't Base"
    );
    let warnings: Vec<_> = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Warning)
        .collect();
    assert_eq!(
        warnings.len(),
        warnings_before + 1,
        "must log a Warning entry naming the drop"
    );
    assert!(
        warnings.last().unwrap().text.contains("insert"),
        "the warning must name the mode that refused it, got: {:?}",
        warnings.last().unwrap().text
    );
}

#[test]
fn show_menu_accepted_in_normal_mode() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::host::UiHost;

    let mut ed = editor_from("-[x]>abcdefgh\n");
    let pane = focused_pane(&ed);
    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    let result = host.show_menu(
        pane,
        script_rows(&["a"]),
        hume_scripting::Callback::for_test(steel::rvals::SteelVal::Void),
    );
    assert!(result.is_ok());
    assert!(ed.state.input.menu().is_some());
}

/// Same async-staleness rule as `show_menu_from_insert_…` above, triggered
/// the other way: the mode layer is still `Base`, but a picker landed on
/// top of it while the `codeAction` response was in flight. `show-menu!`
/// must not bury a picker under a menu it never asked for. It drops
/// silently, same as the mode-changed case.
#[test]
fn show_menu_drops_silently_when_a_picker_is_open() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_core::Severity;
    use hume_scripting::host::{PickerOpts, UiHost};

    let mut ed = editor_from("-[x]>abcdefgh\n");
    let session = crate::editor::input_stack::picker::PickerSession::new(
        hume_scripting::Callback::for_test(steel::rvals::SteelVal::BoolV(false)),
        PickerOpts::default(),
        Vec::new(),
    );
    crate::editor::input_stack::picker::open_picker(&mut ed.state, &ed.view, session)
        .expect("no `.` is in progress");

    let traces_before = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Trace)
        .count();

    let pane = focused_pane(&ed);
    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    let result = host.show_menu(
        pane,
        script_rows(&["a"]),
        hume_scripting::Callback::for_test(steel::rvals::SteelVal::Void),
    );
    assert!(
        result.is_ok(),
        "a stale async response must never error: it would abort the whole call batch"
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

/// A drawer is non-modal (`DrawerLayer::is_modal() == false`): it's built to
/// be worked over (a stray key falls through and it stays open), so an open
/// drawer must not read as "the stack moved" the way a picker does
/// above: a code-action menu must still open while the user is browsing
/// diagnostics in the drawer.
///
/// `InputStack::is_settled_for` counts only a *modal* overlay as the stack
/// moving, so the drawer leaves the menu free to open.
#[test]
fn show_menu_opens_over_an_open_drawer() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::host::UiHost;

    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda (pane)
             (show-drawer-list! pane (map (lambda (row) (cons row row)) (list "one.rs:1")) (lambda (row tok) (void)))))"#,
    );
    type_cmd(&mut ed, ":go");
    assert!(ed.state.input.drawer().is_some(), "sanity: drawer open");

    let pane = focused_pane(&ed);
    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    let result = host.show_menu(
        pane,
        script_rows(&["a"]),
        hume_scripting::Callback::for_test(steel::rvals::SteelVal::Void),
    );

    assert!(result.is_ok());
    assert!(
        ed.state.input.menu().is_some(),
        "the menu must open over the non-modal drawer"
    );
    assert!(
        ed.state.input.drawer().is_some(),
        "the drawer must survive underneath it"
    );
}

/// A second `show-menu!` response while the first menu is still open (two
/// `lsp-code-action` responses racing) must replace it (a refresh, not
/// staleness), firing the outgoing menu's callback with `#f` via ordinary
/// teardown, same shape `show-drawer-list!`'s own self-replace already has.
///
/// `open_menu`'s self-replace exception reads a `Menu` already open as a
/// refresh, not staleness, and retires it through `take_firing_false`.
#[test]
fn show_menu_replaces_a_menu_already_open_and_fires_its_callback() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::host::UiHost;

    let mut ed = editor_from("-[x]>abcdefgh\n");
    let pane = focused_pane(&ed);
    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.show_menu(
        pane,
        script_rows(&["old"]),
        hume_scripting::Callback::for_test(steel::rvals::SteelVal::Void),
    )
    .unwrap();
    assert!(ed.state.input.menu().is_some(), "sanity: first menu open");

    let pane = focused_pane(&ed);
    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    let result = host.show_menu(
        pane,
        script_rows(&["new-a", "new-b"]),
        hume_scripting::Callback::for_test(steel::rvals::SteelVal::Void),
    );

    assert!(result.is_ok());
    assert_eq!(
        ed.state.input.menu().unwrap().rows.len(),
        2,
        "the fresh item list must have replaced the old one"
    );
    assert!(
        matches!(
            ed.state.config.pending_work.front(),
            Some(crate::editor::event::PendingWork::Call { call, .. })
                if matches!(call.args(), [arg] if matches!(arg.value(), steel::rvals::SteelVal::BoolV(false)))
        ),
        "the outgoing menu's callback must fire with #f"
    );
}

// ── Render snapshot: highlighted row ──────────────────────────────────────────

#[test]
fn selected_row_renders_with_the_menu_selected_scope() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = open_headless(None);
    ed.view.theme = crate::testing::build_snapshot_theme();
    ed.feed_key(key('i'));
    for ch in "abcdefgh".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_esc());
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda (pane)
             (show-menu! pane (list (cons "Extract function" 'extract)
                                    (cons "Inline variable" 'inline))
               (lambda (choice) (void)))))"#,
    );
    type_cmd(&mut ed, ":go");

    let mut ctx = RenderContext::new();
    ed.resize_terminal(40, 10);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    let rect = Rect::new(0, 0, 40, 10);
    let snap = render_snapshot::render_to_styled_string(&mut ed, rect);
    insta::assert_snapshot!(snap);
}
