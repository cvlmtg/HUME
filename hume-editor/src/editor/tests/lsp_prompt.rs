// Steel minibuffer prompt: (prompt! label
// on-confirm #:prefill text), (symbol-under-cursor bid).

use super::*;
use crate::editor::commands::open_pane_in_layout;
use hume_scripting::ScriptingHost;

#[test]
fn prompt_confirm_calls_callback_with_typed_text() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda ()
             (prompt! "Name: " (lambda (s) (log! 'info (to-string s))))))"#,
    );
    type_cmd(&mut ed, ":go");
    assert_eq!(ed.state.mode(), hume_engine::types::EditorMode::Command);

    ed.feed_key(key('h'));
    ed.feed_key(key('i'));
    ed.feed_key(key_enter());
    ed.settle();

    assert_eq!(ed.state.status_msg.clone().unwrap(), "hi");
    assert_eq!(ed.state.mode(), hume_engine::types::EditorMode::Normal);
    assert!(ed.state.minibuf().is_none());
}

#[test]
fn prompt_esc_calls_callback_with_false_exactly_once() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda ()
             (prompt! "Name: " (lambda (s) (log! 'info (to-string s))))))"#,
    );
    type_cmd(&mut ed, ":go");

    ed.feed_key(key('h'));
    ed.feed_key(key_esc());
    ed.settle();

    assert_eq!(ed.state.status_msg.clone().unwrap(), "#false");
    assert!(ed.state.minibuf().is_none());
}

#[test]
fn prompt_prefill_is_visible_and_editable() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda ()
             (prompt! "Name: " (lambda (s) (log! 'info (to-string s))) #:prefill "old")))"#,
    );
    type_cmd(&mut ed, ":go");

    let mb = ed.state.minibuf().unwrap();
    assert_eq!(mb.input, "old");
    assert_eq!(
        mb.cursor,
        "old".len(),
        "cursor must start at the end of the prefill"
    );

    // Editable: Backspace removes the trailing char, then typing appends.
    ed.feed_key(key_backspace());
    ed.feed_key(key('a'));
    ed.feed_key(key_enter());
    ed.settle();
    assert_eq!(ed.state.status_msg.clone().unwrap(), "ola");
}

#[test]
fn second_prompt_while_one_is_open_errors() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda ()
             (prompt! "a" (lambda (s) (log! 'info "cb1")))
             (prompt! "b" (lambda (s) (log! 'info "cb2")))))"#,
    );
    type_cmd(&mut ed, ":go");

    // The first prompt! already took effect (Steel errors don't roll back
    // prior host mutations within the same command body).
    assert_eq!(ed.state.minibuf().unwrap().prompt, "a");
    // The command's overall failure (from the second prompt!'s error) is
    // reported rather than silently swallowed.
    let msg = ed.state.status_msg.clone().unwrap_or_default();
    assert!(
        msg.to_lowercase().contains("already open") || msg.to_lowercase().contains("error"),
        "expected an error message, got {msg:?}"
    );
}

/// A `Prompt` buried under a `Drawer` (`show-drawer-list!` opens first,
/// landing below — it isn't a mode layer, so `prompt!`'s `push_mode_layer`
/// never truncates it; `prompt!` then lands above it) must still fire its
/// callback with `#f` when `close-drawer!` truncates the drawer and takes
/// the buried prompt with it as collateral — the same "exactly one call
/// fires, on Confirm or on any cancel path" contract this file's header
/// promises for every other retirement.
///
/// Fail oracle: before this fix, `PromptLayer::tear_down` only reset
/// history — the callback was silently dropped and `pending_work` would be
/// empty below.
#[test]
fn close_drawer_through_a_buried_prompt_still_fires_its_callback() {
    use crate::editor::host_impl::EditorHostImpl;
    use hume_scripting::host::UiHost;

    let mut ed = editor_from("-[x]>abcdefgh\n");
    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.show_drawer_list(vec!["a".to_string()], steel::rvals::SteelVal::Void)
        .unwrap();
    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.prompt(
        "Name: ".to_string(),
        String::new(),
        steel::rvals::SteelVal::Void,
    )
    .unwrap();
    assert!(ed.state.input.drawer().is_some(), "sanity: drawer open");
    assert!(ed.state.minibuf().is_some(), "sanity: prompt open above it");

    let mut host = EditorHostImpl::new(&mut ed.state, &mut ed.view);
    host.close_drawer().unwrap();

    assert!(ed.state.input.drawer().is_none());
    assert!(ed.state.minibuf().is_none(), "the prompt is gone too");
    assert!(
        matches!(
            ed.state.config.pending_work.front(),
            Some(crate::editor::event::PendingWork::Call(_, args))
                if matches!(args.as_slice(), [steel::rvals::SteelVal::BoolV(false)])
        ),
        "the buried prompt's callback must still fire with #f"
    );
}

/// `EditorState::minibuf()` (the statusline row / hardware cursor's own
/// reader) must not resolve a `Prompt` buried under a `Picker` — the picker
/// owns the keyboard and paints over everything else, so painting a dead
/// prompt row underneath it would show state nothing can act on.
///
/// Fail oracle: before this fix, `EditorState::minibuf()` delegated to
/// `InputStack::minibuf()` (topmost-of-any-depth), so the first assertion
/// below would still find the buried prompt.
#[test]
fn minibuf_does_not_resolve_a_prompt_buried_under_a_picker() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda ()
             (prompt! "Name: " (lambda (s) (void)))))"#,
    );
    type_cmd(&mut ed, ":go");
    assert!(ed.state.minibuf().is_some(), "sanity: prompt owns the row");

    let session = crate::editor::input_stack::picker::PickerSession::new(
        steel::rvals::SteelVal::BoolV(false),
        hume_scripting::host::PickerOpts::default(),
    );
    crate::editor::input_stack::picker::open_picker(&mut ed.state, &ed.view, session);

    assert!(
        ed.state.minibuf().is_none(),
        "the buried prompt must no longer own the row"
    );
    assert!(
        ed.state.input.minibuf().is_some(),
        "sanity: the prompt itself is still on the stack, merely buried"
    );
}

#[test]
fn prompt_mode_round_trips_and_fires_on_mode_change() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdefghij\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda ()
             (prompt! "x: " (lambda (s) (void)))))
           (register-hook! 'on-mode-change (lambda (old new) (call! "move-right")))"#,
    );

    let before = state(&ed);
    type_cmd(&mut ed, ":go");
    ed.settle();
    assert_eq!(
        ed.state.mode(),
        hume_engine::types::EditorMode::Command,
        "prompt! must reuse Command mode, not a new EditorMode"
    );
    let after_enter = state(&ed);
    assert_ne!(
        before, after_enter,
        "on-mode-change must fire entering the prompt"
    );

    ed.feed_key(key_esc());
    ed.settle();
    assert_eq!(ed.state.mode(), hume_engine::types::EditorMode::Normal);
    assert_ne!(
        state(&ed),
        after_enter,
        "on-mode-change must fire again leaving the prompt"
    );
}

/// `push_mode_layer` truncates the outgoing mode layer before pushing —
/// entering `Prompt` from `Insert` (a queued `(after 0 …)` thunk firing
/// while the user is mid-insert) must end the insert session cleanly
/// (edit group committed, `insert_session` cleared) rather than leaving it
/// dangling underneath a `Prompt` layer with no way back to it.
#[test]
fn prompt_from_insert_ends_the_insert_session_cleanly() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bc\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "arm" "" (lambda ()
             (after 0 (lambda () (prompt! "x: " (lambda (s) (void)))))))"#,
    );
    type_cmd(&mut ed, ":arm");

    ed.feed_key(key('i'));
    ed.feed_key(key('X'));
    assert_eq!(ed.state.mode(), hume_engine::types::EditorMode::Insert);
    assert!(
        ed.state.insert_session.is_some(),
        "sanity: an insert session is open"
    );
    assert_eq!(ed.doc().text().to_string(), "Xabc\n");

    ed.settle(); // drains the due timer, which calls prompt! mid-insert
    assert_eq!(
        ed.state.mode(),
        hume_engine::types::EditorMode::Command,
        "prompt! must have taken over the mode layer"
    );
    assert!(
        ed.state.insert_session.is_none(),
        "the insert session must be finalized, not left dangling under Prompt"
    );
    assert_eq!(
        ed.doc().text().to_string(),
        "Xabc\n",
        "the typed char must have committed, not been lost"
    );
}

/// Same truncate-then-push as the Insert case above, for `Search`: a
/// `Prompt` landing mid-search (before the pattern is confirmed) must run
/// `Search`'s own teardown — restoring the pre-search selection and
/// clearing the live pattern — exactly as `Esc` would, leaving nothing
/// stale behind once `Prompt` takes over.
#[test]
fn prompt_from_search_restores_pre_search_selection_and_clears_the_pattern() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[h]>ello world\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "arm" "" (lambda ()
             (after 0 (lambda () (prompt! "x: " (lambda (s) (void)))))))"#,
    );
    type_cmd(&mut ed, ":arm");

    ed.feed_key(key('/'));
    for ch in "world".chars() {
        ed.feed_key(key(ch));
    }
    assert_eq!(ed.state.mode(), hume_engine::types::EditorMode::Search);
    assert_eq!(
        state(&ed),
        "hello -[world]>\n",
        "sanity: live search has already moved the selection"
    );

    ed.settle(); // drains the due timer, which calls prompt! mid-search
    assert_eq!(
        ed.state.mode(),
        hume_engine::types::EditorMode::Command,
        "prompt! must have taken over the mode layer"
    );
    assert_eq!(
        state(&ed),
        "-[h]>ello world\n",
        "the selection must be restored to its pre-search position"
    );
    assert!(
        ed.search_pattern().is_none(),
        "the in-progress (unconfirmed) pattern must be cleared, not left stale"
    );
}

/// `exit-insert` reached from outside Insert (a queued `(after 0 …)` thunk,
/// same as a hook or async LSP callback would) must be a no-op — it has no
/// `Insert` layer to end, so it must not cancel an unrelated `Prompt`
/// session that happens to be the current mode layer. `cmd_exit_insert` is
/// a registered mappable command reachable via `(call! "exit-insert")` from
/// any mode, not gated on Insert actually being current.
///
/// Fail oracle: if `end_insert_session` truncated `mode_layer()`
/// unconditionally (whatever layer that happens to be) instead of looking
/// up the `Insert` layer by kind, this call would truncate `Prompt` instead
/// — running its `tear_down` (which never fires a Steel callback) instead
/// of `finish_steel_prompt`, so `on-confirm` would never fire and the
/// assertions below would fail.
#[test]
fn exit_insert_outside_insert_does_not_cancel_an_unrelated_prompt() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[x]>\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "go" "" (lambda ()
             (after 0 (lambda () (call! "exit-insert")))
             (prompt! "Name: " (lambda (s) (log! 'info (to-string s))))))"#,
    );
    type_cmd(&mut ed, ":go");
    assert!(ed.state.minibuf().is_some(), "sanity: prompt is open");
    assert!(
        ed.state.status_msg.is_none(),
        "sanity: on-confirm hasn't fired yet"
    );

    ed.settle(); // drains the due timer, which calls exit-insert

    assert!(
        ed.state.minibuf().is_some(),
        "exit-insert outside Insert must not cancel the open prompt"
    );
    assert!(
        ed.state.status_msg.is_none(),
        "the prompt's on-confirm must not fire from an unrelated exit-insert"
    );
}

// ── symbol-under-cursor ──────────────────────────────────────────────────────

#[test]
fn symbol_under_cursor_on_a_word_char_returns_the_whole_word() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("foo -[b]>ar baz\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "check" "" (lambda ()
             (log! 'info (symbol-under-cursor (current-buffer)))))"#,
    );
    type_cmd(&mut ed, ":check");
    assert_eq!(ed.state.status_msg.clone().unwrap(), "bar");
}

#[test]
fn symbol_under_cursor_on_whitespace_returns_empty() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("foo-[ ]>bar\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "check" "" (lambda ()
             (log! 'info (to-string "[" (symbol-under-cursor (current-buffer)) "]"))))"#,
    );
    type_cmd(&mut ed, ":check");
    assert_eq!(ed.state.status_msg.clone().unwrap(), "[  ]");
}

#[test]
fn symbol_under_cursor_on_punctuation_returns_empty() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("foo-[.]>bar\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "check" "" (lambda ()
             (log! 'info (to-string "[" (symbol-under-cursor (current-buffer)) "]"))))"#,
    );
    type_cmd(&mut ed, ":check");
    assert_eq!(ed.state.status_msg.clone().unwrap(), "[  ]");
}

#[test]
fn symbol_under_cursor_finds_a_word_in_a_non_focused_pane() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("foo -[b]>ar baz\n");

    // Open a second buffer in a second pane, then focus that pane — `bid`
    // stays open (with its "bar" cursor) in the now-unfocused first pane.
    let extra = tmp.path().join("other.rs");
    std::fs::write(&extra, "fn other() {}\n").unwrap();
    ed.open_extra_file(&extra);
    let other_bid = ed
        .state
        .buffers
        .find_by_path(&std::fs::canonicalize(&extra).unwrap())
        .expect("extra file must be open in the buffer list");
    let start_pid = ed.state.focus.id();
    let other_pid = open_pane_in_layout(
        &mut ed.state,
        &mut ed.view,
        start_pid,
        other_bid,
        hume_engine::pipeline::Direction::Horizontal,
    )
    .unwrap();
    ed.state.focus.set_for_test(other_pid);

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(let ((hidden (car (filter (lambda (b) (not (equal? b (current-buffer)))) (buffers)))))
             (equal? (symbol-under-cursor hidden) "bar"))"#,
    );
    assert!(
        fired,
        "symbol-under-cursor must resolve bid in whichever pane currently shows it, not just the focused one"
    );
}

#[test]
fn symbol_under_cursor_is_empty_once_no_pane_shows_the_buffer() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("foo -[b]>ar baz\n");

    // Redirect the sole pane to a different buffer. `bid`'s PaneBufferState
    // stays seeded (stale) in that pane's map — `pane_state::ensure` never
    // removes an entry — but the pane's live `buffer_id` no longer points
    // at it, so no pane currently shows it.
    let extra = tmp.path().join("other.rs");
    std::fs::write(&extra, "fn other() {}\n").unwrap();
    ed.open_extra_file(&extra);
    let other_bid = ed
        .state
        .buffers
        .find_by_path(&std::fs::canonicalize(&extra).unwrap())
        .expect("extra file must be open in the buffer list");
    ed.switch_to_buffer_with_jump(other_bid);

    let fired = run_probe(
        &mut ed,
        ScriptingHost::new(),
        tmp.path(),
        r#"(let ((hidden (car (filter (lambda (b) (not (equal? b (current-buffer)))) (buffers)))))
             (equal? (symbol-under-cursor hidden) ""))"#,
    );
    assert!(
        fired,
        "symbol-under-cursor must return \"\" once the buffer is shown in no pane, not the stale cursor's word"
    );
}
