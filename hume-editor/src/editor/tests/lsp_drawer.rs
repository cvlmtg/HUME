// Bottom drawer: (show-drawer-list!
// items on-select) / (close-drawer!), the Ctrl-d/Ctrl-u single-step input
// handling in `drawer_input`, and the engine chrome band (see `hume-engine`'s
// `pane_area_*` tests for the partition math itself).

use hume_grid::Rect;
use std::path::Path;

use super::*;
use hume_engine::pipeline::RenderContext;

fn arm_three_items(ed: &mut Editor, tmp: &Path) {
    run(
        ed,
        tmp,
        r#"(define-typed-command! "go" "" (lambda ()
             (show-drawer-list! (list "one.rs:1" "two.rs:2" "three.rs:3")
               (lambda (idx) (log! 'info (to-string idx))))))"#,
    );
    type_cmd(ed, ":go");
}

// ── show-drawer-list! / close-drawer! ─────────────────────────────────────────

#[test]
fn show_drawer_list_populates_model_and_view() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());

    assert!(ed.state.input.drawer().is_some());
    let guard = ed.state.views.drawer.read();
    let view = guard.as_ref().expect("view must be populated on open");
    assert_eq!(*view.rows, vec!["one.rs:1", "two.rs:2", "three.rs:3"]);
    assert_eq!(view.selected, 0);
}

/// A drawer opening while a `Scrollable` popup is up must retire the popup
/// first — `DrawerLayer::setup` runs `InputStack::clear_popups` before
/// landing, keeping `PopupLayer`'s "never buried" invariant true. A
/// `Drawer` is non-modal, so nothing else in `show-drawer-list!`'s own gate
/// (`is_settled_for`) would otherwise stop it landing directly above the
/// popup.
///
/// Calls `EditorHostImpl::show_drawer_list` directly rather than typing a
/// `:` command to trigger it — entering Command mode for the keystroke
/// itself would clear the popup via `push_mode_layer`'s own unconditional
/// clear, before `show-drawer-list!` ever ran, masking exactly the
/// behavior this test exists to pin.
///
/// Fail oracle: before the fix, the drawer landed above the popup instead
/// of clearing it — `ed.state.input.popup()` would still read `Some` here.
#[test]
fn show_drawer_list_over_a_live_popup_clears_it() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::host::{PopupKind, UiHost};

    let mut ed = editor_from("-[x]>abcdefgh\n");
    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.show_popup("hi".to_string(), PopupKind::Scrollable, false, None)
        .expect("show-popup! must succeed");
    assert!(ed.state.input.popup().is_some(), "sanity: popup open");

    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.show_drawer_list(
        vec!["one".to_string()],
        steel::rvals::SteelVal::BoolV(false),
    )
    .expect("show-drawer-list! must succeed");

    assert!(ed.state.input.drawer().is_some(), "the drawer opened");
    assert!(
        ed.state.input.popup().is_none(),
        "the popup must be cleared, not buried, when the drawer lands above it"
    );
}

/// Same async-staleness rule as `show_drawer_list_from_insert_…` below,
/// triggered the other way: the mode layer is still `Base`, but a picker
/// landed on top of it while the references response was in flight.
/// `show-drawer-list!` must not bury a picker under a drawer it never asked
/// for — it drops silently, same as the mode-changed case.
#[test]
fn show_drawer_list_drops_silently_when_a_picker_is_open() {
    use crate::editor::Severity;
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::host::{PickerOpts, UiHost};

    let mut ed = editor_from("-[x]>abcdefgh\n");
    let session = crate::editor::input_stack::picker::PickerSession::new(
        steel::rvals::SteelVal::BoolV(false),
        PickerOpts::default(),
    );
    crate::editor::input_stack::picker::open_picker(&mut ed.state, &ed.view, session);

    let traces_before = ed
        .state
        .message_log
        .entries()
        .filter(|e| e.severity == Severity::Trace)
        .count();

    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    let result = host.show_drawer_list(vec!["a".to_string()], steel::rvals::SteelVal::Void);
    assert!(
        result.is_ok(),
        "a stale async response must never error — it would abort the whole call batch"
    );
    assert!(
        ed.state.input.drawer().is_none(),
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

/// `push_mode_layer` pushes a new mode layer *above* whatever overlay
/// already sits on `Base` — so `i` while a drawer is open lands `Insert`
/// on top of it (dispatch reaches `Insert`, not the drawer, for every key
/// typed), and `Esc` ending Insert truncates only its own layer, leaving
/// the drawer exactly where it was: still open, still driving Ctrl-d/
/// Ctrl-u and the rest of its own keys.
#[test]
fn insert_above_an_open_drawer_then_esc_leaves_the_drawer_fully_functional() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());
    assert!(ed.state.input.drawer().is_some(), "sanity: drawer open");

    ed.feed_key(key('i'));
    assert_eq!(
        ed.state.mode(),
        Mode::Insert,
        "i must enter Insert above the drawer"
    );
    ed.feed_key(key('X'));
    assert_eq!(ed.doc().text().to_string(), "Xxabcdefgh\n");

    ed.feed_key(key_esc());
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert!(
        ed.state.input.drawer().is_some(),
        "the drawer must have survived the Insert round trip untouched"
    );

    // The drawer must still be driving its own keys, not just present.
    ed.feed_key(key_ctrl('d'));
    assert_eq!(
        ed.state.input.drawer().unwrap().selected,
        1,
        "Ctrl-d must still move the drawer's selection"
    );
}

/// `show-drawer-list!` from Insert is a benign timing issue (a references
/// response landing after the user left Normal), not a plugin bug — it
/// drops silently (`Ok`, a `Trace` log entry) rather than erroring, so it
/// never aborts the `run_call_batch` a real async callback is batched into.
/// Same async-staleness rule as the picker case above, checked the other
/// way: the mode layer itself has moved, rather than the stack above it.
#[test]
fn show_drawer_list_from_insert_drops_silently_as_a_mode_layer_race() {
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
    let result = host.show_drawer_list(vec!["a".to_string()], steel::rvals::SteelVal::Void);
    assert!(
        result.is_ok(),
        "a mode-layer race must never error — it would abort the whole call batch"
    );
    assert!(
        ed.state.input.drawer().is_none(),
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

/// `sync_drawer_view` runs unconditionally every frame while the drawer is
/// open (the self-healing backstop this module's header comment describes),
/// so its row list must be shared (`Arc::clone`) rather than deep-copied —
/// a references batch can carry thousands of rows, and a plain `Vec::clone`
/// there would reallocate all of them every frame for as long as the drawer
/// stays open.
///
/// Sabotage oracle: revert `DrawerLayer::items` and `DrawerViewState::rows`
/// to a bare `Vec<String>` (with `sync_drawer_view` deep-cloning it, as
/// before) — this test fails to compile, since there is no `Arc` left on
/// either side for `Arc::ptr_eq` to compare.
#[test]
fn drawer_view_shares_the_model_s_row_list_instead_of_cloning_it() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());

    let model_items = std::sync::Arc::clone(&ed.state.input.drawer().unwrap().items);
    let view_rows = {
        let guard = ed.state.views.drawer.read();
        std::sync::Arc::clone(&guard.as_ref().unwrap().rows)
    };
    assert!(
        std::sync::Arc::ptr_eq(&model_items, &view_rows),
        "the view's row list must be the same allocation as the model's, \
         not an independent copy"
    );

    // A fresh `show-drawer-list!` call replaces the model's `Arc` wholesale;
    // the next sync must follow it to a *different* allocation, not keep
    // pointing at the first one.
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "refresh" "" (lambda ()
             (show-drawer-list! (list "replaced")
               (lambda (idx) (void)))))"#,
    );
    type_cmd(&mut ed, ":refresh");
    let view_rows_after = {
        let guard = ed.state.views.drawer.read();
        std::sync::Arc::clone(&guard.as_ref().unwrap().rows)
    };
    assert!(
        !std::sync::Arc::ptr_eq(&view_rows, &view_rows_after),
        "the view must follow a replaced model to its new row list"
    );
    assert_eq!(*view_rows_after, vec!["replaced"]);
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
    assert!(ed.state.input.drawer().is_some());

    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.close_drawer().unwrap();

    assert!(ed.state.input.drawer().is_none());
    assert!(ed.state.views.drawer.read().is_none());
    assert!(
        ed.state.config.pending_work.is_empty(),
        "close_drawer must not queue the callback"
    );
}

/// `close-drawer!` while the drawer is buried under `Insert` (browsing while
/// editing, `insert_above_an_open_drawer_…` above) is the drawer's *normal*
/// state, not a "wrong widget is active" mistake — it must close the
/// drawer wherever it sits on the stack, same as every Rust-internal
/// retirement (`buffer::disk`/`buffer::lifecycle`'s confirm-on-close checks,
/// `lsp_stop_one`'s own doc) already does, rather than erroring because it
/// isn't `top()`. `truncate` is the only removal op, so this takes `Insert`
/// with it too — same "closing a buried widget takes everything above it"
/// contract `lsp_stop_one` already accepts for a `Completion` a picker has
/// landed on; the collateral `Insert` layer still gets ordinary teardown
/// (`tear_down_insert` commits its edit group), not a silent drop.
///
/// Fail oracle: before this fix, `close_drawer` returned `Err` whenever the
/// drawer was present but not `top()` — `.unwrap()` below would panic.
#[test]
fn close_drawer_closes_a_drawer_buried_under_insert() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::host::UiHost;

    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());
    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    assert_eq!(ed.state.mode(), Mode::Insert, "sanity: Insert above drawer");
    assert!(ed.state.input.drawer().is_some(), "sanity: drawer buried");

    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.close_drawer().unwrap();

    assert!(ed.state.input.drawer().is_none(), "the drawer must be gone");
    assert_eq!(
        ed.state.mode(),
        Mode::Normal,
        "Insert goes with it — truncate is the only removal op"
    );
    assert_eq!(
        ed.doc().text().to_string(),
        "Xxabcdefgh\n",
        "the typed char must have committed via ordinary Insert teardown"
    );
}

/// A code-action menu opened above an open references drawer (explicitly
/// supported — `DrawerLayer::is_modal` returning `false` is exactly what
/// lets a menu open while the user browses one) is collateral, not the
/// named target, when `close-drawer!` closes the drawer beneath it.
///
/// Fail oracle: before `Layer::tear_down` gained its `Removal` reason,
/// `MenuLayer::tear_down` was unconditionally silent — the menu's callback
/// was dropped forever, neither an index nor `#f`. `ed.state.status_msg`
/// would stay `None` below.
#[test]
fn close_drawer_fires_the_menu_s_callback_when_a_menu_sits_above_it() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::host::UiHost;

    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go-drawer" "" (lambda ()
             (show-drawer-list! (list "one.rs:1") (lambda (idx) (log! 'info (to-string idx))))))
           (define-typed-command! "go-menu" "" (lambda ()
             (show-menu! (list "Extract function") (lambda (idx) (log! 'info (to-string idx))))))"#,
    );
    type_cmd(&mut ed, ":go-drawer");
    assert!(ed.state.input.drawer().is_some(), "sanity: drawer open");
    type_cmd(&mut ed, ":go-menu");
    assert!(
        ed.state.input.menu().is_some(),
        "sanity: menu open above the drawer"
    );

    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.close_drawer().unwrap();
    ed.settle();

    assert!(ed.state.input.drawer().is_none(), "the drawer must be gone");
    assert!(
        ed.state.input.menu().is_none(),
        "the menu goes with it — collateral"
    );
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "#false",
        "the menu's callback must fire with #f, not be dropped silently"
    );
}

// ── Self-replace fires #f to the outgoing callback ───────────────────────────

/// A second `show-drawer-list!` while one is open replaces it — and the
/// outgoing drawer owner must learn its drawer is gone via `#f` (the same
/// shape as `show-menu!`'s own self-replace). The `:diagnostics` drawer
/// refresh relies on this to drop its open-tracking instead of refreshing a
/// dead drawer over someone else's list.
///
/// Fail oracle: before the fix the replace was silent — `status_msg` would
/// still be `None` below instead of `"#false"`.
#[test]
fn replace_fires_false_to_the_outgoing_callback() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda ()
             (show-drawer-list! (list "a" "b") (lambda (idx) (log! 'info (to-string idx))))))
           (define-typed-command! "other" "" (lambda ()
             (show-drawer-list! (list "c") (lambda (idx) (log! 'info "second")))))"#,
    );
    // No settle between the two opens: back-to-back shows are the normal
    // self-replace path (see the `Arc` test above). (The second command is
    // named `other`, not `go2`: a trailing digit on a typed command is
    // parsed as a count, which would just re-run `go`.)
    type_cmd(&mut ed, ":go");
    assert!(
        ed.state.status_msg.is_none(),
        "opening must not fire the callback"
    );

    type_cmd(&mut ed, ":other");
    ed.settle();

    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "#false",
        "the outgoing drawer's callback must fire with #f on replace"
    );
    {
        let guard = ed.state.views.drawer.read();
        assert_eq!(*guard.as_ref().unwrap().rows, vec!["c"]);
    }
}

// ── update-drawer-list! / drawer-selected-index ──────────────────────────────

/// `update-drawer-list!` replaces rows + callback in place (no reset to row
/// 0, no `#f` to the outgoing callback — it is a refresh, not a replace),
/// clamps an out-of-range selection, and reports whether it applied.
#[test]
fn update_replaces_rows_callback_and_selection_in_place() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda ()
             (show-drawer-list! (list "a" "b" "c") (lambda (idx) (log! 'info "old")))))
           (define-typed-command! "sel" "" (lambda ()
             (log! 'info (to-string (drawer-selected-index)))))
           (define-typed-command! "upd" "" (lambda ()
             (log! 'info (to-string (update-drawer-list! (list "x" "y")
               (lambda (idx) (log! 'info (to-string idx))) 1)))))
           (define-typed-command! "upd-big" "" (lambda ()
             (log! 'info (to-string (update-drawer-list! (list "x" "y")
               (lambda (idx) (log! 'info (to-string idx))) 99)))))"#,
    );
    type_cmd(&mut ed, ":go");
    ed.settle();

    ed.feed_key(key_ctrl('d'));
    type_cmd(&mut ed, ":sel");
    ed.settle();
    assert_eq!(ed.state.status_msg.clone().unwrap(), "1");
    assert_eq!(
        ed.state.input.drawer().unwrap().selected,
        1,
        "typing a command must not disturb the drawer selection"
    );

    type_cmd(&mut ed, ":upd");
    ed.settle();
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "#true",
        "update must report it applied"
    );
    let drawer = ed.state.input.drawer().unwrap();
    assert_eq!(*drawer.items, vec!["x", "y"]);
    assert_eq!(
        drawer.selected, 1,
        "caller's selection must be kept, not reset"
    );
    {
        let guard = ed.state.views.drawer.read();
        assert_eq!(*guard.as_ref().unwrap().rows, vec!["x", "y"]);
        assert_eq!(guard.as_ref().unwrap().selected, 1);
    }

    // The new callback is live: Enter fires it with the kept selection, and
    // the drawer stays open.
    ed.feed_key(key_enter());
    ed.settle();
    assert_eq!(ed.state.status_msg.clone().unwrap(), "1");
    assert!(ed.state.input.drawer().is_some());

    // Out-of-range selection clamps to the last row, same as the Ctrl-d keys.
    type_cmd(&mut ed, ":upd-big");
    ed.settle();
    assert_eq!(ed.state.status_msg.clone().unwrap(), "#true");
    assert_eq!(ed.state.input.drawer().unwrap().selected, 1);
}

/// With no drawer open both new builtins degrade to `#f`: the update applies
/// nothing (and opens nothing), the getter reports no selection. Never an
/// error — a closed-or-replaced drawer is an expected-normal race.
#[test]
fn update_and_selected_index_with_none_open_report_false() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "upd" "" (lambda ()
             (log! 'info (to-string (update-drawer-list! (list "x")
               (lambda (idx) (void)) 0)))))
           (define-typed-command! "sel" "" (lambda ()
             (log! 'info (to-string (drawer-selected-index)))))"#,
    );
    type_cmd(&mut ed, ":upd");
    ed.settle();
    assert_eq!(ed.state.status_msg.clone().unwrap(), "#false");
    assert!(
        ed.state.input.drawer().is_none(),
        "an update must never open a drawer"
    );

    type_cmd(&mut ed, ":sel");
    ed.settle();
    assert_eq!(ed.state.status_msg.clone().unwrap(), "#false");
}

/// An empty `show-drawer-list!` is rejected — a 0-row drawer would leave
/// `Enter` firing `0` with no row behind it.
///
/// Fail oracle: before the fix the empty drawer opened with `selected` 0.
#[test]
fn show_with_empty_items_errors_and_opens_nothing() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::host::UiHost;

    let mut ed = editor_from("-[x]>abcdefgh\n");
    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    let result = host.show_drawer_list(Vec::new(), steel::rvals::SteelVal::BoolV(false));
    assert!(
        result.is_err(),
        "empty show must fail fast instead of opening a 0-row drawer"
    );
    assert!(
        ed.state.input.drawer().is_none(),
        "a rejected open must leave no drawer behind"
    );
    assert!(
        ed.state.views.drawer.read().is_none(),
        "the view must stay empty too"
    );
}

/// An empty `update-drawer-list!` is a no-op `#f` — callers close instead
/// of clearing through an update.
///
/// Fail oracle: before the fix the update applied (`#true`) and wiped the
/// rows, leaving a 0-row drawer where `Enter` would fire `0`.
#[test]
fn update_with_empty_items_is_a_noop_false() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda ()
             (show-drawer-list! (list "a" "b") (lambda (idx) (void)))))
           (define-typed-command! "upd" "" (lambda ()
             (log! 'info (to-string (update-drawer-list! (list)
               (lambda (idx) (void)) 0)))))"#,
    );
    type_cmd(&mut ed, ":go");
    ed.settle();
    assert!(ed.state.input.drawer().is_some(), "sanity: drawer open");

    type_cmd(&mut ed, ":upd");
    ed.settle();
    assert_eq!(ed.state.status_msg.clone().unwrap(), "#false");
    let drawer = ed.state.input.drawer().unwrap();
    assert_eq!(
        *drawer.items,
        vec!["a".to_string(), "b".to_string()],
        "the old rows must survive a rejected update"
    );
    assert_eq!(drawer.selected, 0);
}

// ── Esc: closes + calls back with #f ─────────────────────────────────────────

#[test]
fn esc_calls_back_with_false_and_closes() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());

    ed.feed_key(key_esc());
    ed.settle();

    assert_eq!(ed.state.status_msg.clone().unwrap(), "#false");
    assert!(ed.state.input.drawer().is_none());
    assert!(ed.state.views.drawer.read().is_none());
}

// ── Enter: fires the callback and stays open, repeatedly ─────────────────────

#[test]
fn enter_calls_back_and_the_drawer_stays_open() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());

    ed.feed_key(key_enter());
    ed.settle();
    assert_eq!(ed.state.status_msg.clone().unwrap(), "0");
    assert!(
        ed.state.input.drawer().is_some(),
        "must stay open after Enter"
    );

    // Move selection and fire again — the callback must still be usable
    // (cloned, not consumed by the first Enter).
    ed.feed_key(key_ctrl('d'));
    ed.feed_key(key_enter());
    ed.settle();
    assert_eq!(ed.state.status_msg.clone().unwrap(), "1");
    assert!(ed.state.input.drawer().is_some());
}

// ── Selection clamps at both ends ─────────────────────────────────────────────

#[test]
fn selection_clamps_at_the_top() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());

    ed.feed_key(key_ctrl('u'));
    ed.feed_key(key_ctrl('u'));
    ed.feed_key(key_enter());
    ed.settle();
    assert_eq!(ed.state.status_msg.clone().unwrap(), "0");
}

#[test]
fn selection_clamps_at_the_bottom() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());

    for _ in 0..5 {
        ed.feed_key(key_ctrl('d'));
    }
    ed.feed_key(key_enter());
    ed.settle();
    assert_eq!(
        ed.state.status_msg.clone().unwrap(),
        "2",
        "clamped, not wrapped"
    );
}

// ── Stray key: neither closes nor invokes, but still executes ────────────────

#[test]
fn stray_key_leaves_the_drawer_open_and_uninvoked_but_still_executes() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_three_items(&mut ed, tmp.path());

    let head_before = ed.current_selections().primary().head();
    ed.feed_key(key('l')); // move-right — not one of the drawer's keys
    ed.settle();

    assert!(
        ed.state.input.drawer().is_some(),
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

/// `j`/`k` and the arrow keys are not drawer keys — they fall through to
/// the source buffer (so vertical motion keeps working while browsing),
/// leaving the drawer open with its selection untouched.
#[test]
fn vertical_motion_keys_fall_through_leaving_the_drawer_selection_untouched() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\ndef\nghi\n");
    arm_three_items(&mut ed, tmp.path());

    let cursor_line = |ed: &Editor| {
        let head = ed.current_selections().primary().head();
        let bid = ed.focused_buffer_id();
        ed.state.buffers.get(bid).text().char_to_line(head)
    };

    ed.feed_key(key('j'));
    ed.settle();
    assert_eq!(
        cursor_line(&ed),
        hume_rope::line::ContentLine::new(1),
        "j must move the source cursor down"
    );

    ed.feed_key(key('k'));
    ed.settle();
    assert_eq!(
        cursor_line(&ed),
        hume_rope::line::ContentLine::new(0),
        "k must move the source cursor back up"
    );

    ed.feed_key(key_down());
    ed.settle();
    assert_eq!(
        cursor_line(&ed),
        hume_rope::line::ContentLine::new(1),
        "Down must move the source cursor down"
    );

    ed.feed_key(key_up());
    ed.settle();
    assert_eq!(
        cursor_line(&ed),
        hume_rope::line::ContentLine::new(0),
        "Up must move the source cursor back up"
    );

    assert!(
        ed.state.input.drawer().is_some(),
        "vertical motion must not close the drawer"
    );
    assert_eq!(
        ed.state.input.drawer().unwrap().selected,
        0,
        "vertical motion must not move the drawer's selection"
    );
    assert!(
        ed.state.status_msg.is_none(),
        "vertical motion must not invoke the callback"
    );
}

#[test]
fn long_list_auto_scrolls_to_keep_selection_visible() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    let items_scm: String = (0..20)
        .map(|i| format!("\"item {i}\""))
        .collect::<Vec<_>>()
        .join(" ");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"(define-typed-command! "go" "" (lambda ()
                 (show-drawer-list! (list {items_scm})
                   (lambda (idx) (log! 'info (to-string idx))))))"#
        ),
    );

    // Populate `last_terminal_area` before any key handling needs it — the
    // scroll clamp reads it to agree with what the engine will next paint.
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 10);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    type_cmd(&mut ed, ":go");

    // capacity = min(20 items + 1, 10 rows / 2 = 5) = 5; visible_rows = 4.
    for _ in 0..6 {
        ed.feed_key(key_ctrl('d'));
    }

    let drawer = ed.state.input.drawer().unwrap();
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

// ── Ctrl-d/Ctrl-u: single-line step ───────────────────────────────────────────

/// Arms the same 20-item / 40×10 fixture as
/// `long_list_auto_scrolls_to_keep_selection_visible`: capacity =
/// min(20+1, 10/2=5) = 5, so `visible_rows` = 4 and each Ctrl-d/Ctrl-u
/// moves exactly one row.
fn arm_twenty_items_in_a_short_terminal(ed: &mut Editor, tmp: &Path) {
    let items_scm: String = (0..20)
        .map(|i| format!("\"item {i}\""))
        .collect::<Vec<_>>()
        .join(" ");
    run(
        ed,
        tmp,
        &format!(
            r#"(define-typed-command! "go" "" (lambda ()
                 (show-drawer-list! (list {items_scm})
                   (lambda (idx) (log! 'info (to-string idx))))))"#
        ),
    );
    // Populate `last_terminal_area` before any key handling needs it — the
    // scroll clamp reads it to agree with what the engine will next paint.
    let mut ctx = RenderContext::new();
    ed.sync_viewport_dims(40, 10);
    ed.settle();
    ed.prepare_frame(&mut ctx);
    type_cmd(ed, ":go");
}

#[test]
fn ctrl_d_steps_down_one_row() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_twenty_items_in_a_short_terminal(&mut ed, tmp.path());

    ed.feed_key(key_ctrl('d'));
    assert_eq!(
        ed.state.input.drawer().unwrap().selected,
        1,
        "a single row, not a half-page"
    );

    // A second step stays inside the first visible window (0..4), so
    // scroll must not advance yet.
    ed.feed_key(key_ctrl('d'));
    let drawer = ed.state.input.drawer().unwrap();
    assert_eq!(drawer.selected, 2);
    assert_eq!(drawer.scroll, 0, "still inside the first window");

    // Stepping past the window (selected = 5) must scroll to keep the new
    // selection in view.
    ed.feed_key(key_ctrl('d'));
    ed.feed_key(key_ctrl('d'));
    ed.feed_key(key_ctrl('d'));
    let drawer = ed.state.input.drawer().unwrap();
    assert_eq!(drawer.selected, 5);
    assert!(
        drawer.scroll > 0,
        "scroll must advance once selection leaves the first window"
    );
}

#[test]
fn ctrl_d_clamps_at_the_last_item() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_twenty_items_in_a_short_terminal(&mut ed, tmp.path());

    for _ in 0..25 {
        ed.feed_key(key_ctrl('d'));
    }
    assert_eq!(
        ed.state.input.drawer().unwrap().selected,
        19,
        "clamped to the last of 20 items, not wrapped or overshot"
    );
}

#[test]
fn ctrl_u_steps_up_one_row() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_twenty_items_in_a_short_terminal(&mut ed, tmp.path());

    ed.feed_key(key_ctrl('d'));
    ed.feed_key(key_ctrl('d')); // selected = 2
    ed.feed_key(key_ctrl('u'));
    assert_eq!(
        ed.state.input.drawer().unwrap().selected,
        1,
        "one row back up from 2"
    );
}

#[test]
fn ctrl_u_clamps_at_the_first_item() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>abcdefgh\n");
    arm_twenty_items_in_a_short_terminal(&mut ed, tmp.path());

    ed.feed_key(key_ctrl('u'));
    let drawer = ed.state.input.drawer().unwrap();
    assert_eq!(drawer.selected, 0, "clamped, never underflows");
    assert_eq!(drawer.scroll, 0);
}

// ── End-to-end: Enter jumps via goto-location!, drawer stays open ────────────

#[test]
fn enter_jump_lands_via_goto_location_and_drawer_stays_open() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\ndef\nghi\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda ()
             (show-drawer-list! (list "line 3")
               (lambda (idx) (goto-location! (list (current-buffer) 2 1))))))"#,
    );
    type_cmd(&mut ed, ":go");

    ed.feed_key(key_enter());
    ed.settle();

    let head = ed.current_selections().primary().head();
    let bid = ed.focused_buffer_id();
    let text = ed.state.buffers.get(bid).text();
    let line = text.char_to_line(head);
    let col = head.chars_since(text.line_to_char(line.into()));
    assert_eq!(
        (line, col),
        (hume_rope::line::ContentLine::new(2), 1),
        "cursor landed at the jump target"
    );
    assert!(
        ed.state.input.drawer().is_some(),
        "drawer stays open after the jump"
    );
}

// ── Render snapshot: drawer band under the shrunk pane grid ──────────────────

#[test]
fn drawer_renders_under_the_pane_with_selected_row_highlighted() {
    let tmp = safe_tempdir();
    let mut ed = Editor::open(None, std::sync::Arc::new(|| {})).unwrap();
    ed.view.theme = crate::testing::build_snapshot_theme();
    ed.feed_key(key('i'));
    for ch in "hello".chars() {
        ed.feed_key(key(ch));
    }
    ed.feed_key(key_esc());
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda ()
             (show-drawer-list! (list "src/a.rs:1: unused import" "src/b.rs:9: TODO")
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
