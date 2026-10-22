//! Dot-repeat of Steel commands and of Insert-key bindings that accept a completion.

use super::*;
use pretty_assertions::assert_eq;

// ── Steel command dot-repeat tests ────────────────────────────────────────────

/// A Steel command registered with `#:repeatable #t` must overwrite
/// the recorded change with its own name so `.` replays the outer
/// Steel body, not the inner native command.
///
/// Buffer is "foo bar\n", initial selection is "foo".
/// Run `del-sel` (repeatable Steel command that calls delete internally) →
/// "foo" is deleted, buffer is " bar\n". Press `w` to select "bar", the
/// first (and only) word on its line, so its leading space is indentation
/// and is never absorbed, and there's no trailing space either (EOL
/// follows), so the default around-word span is bare "bar". Then `.` replays
/// `del-sel` on that selection, leaving " \n".
///
/// If `meta().repeatable` returned `false` for `SteelBacked`,
/// the recorded change would be `None` (no prior recording), so `.` would
/// be a no-op and "bar" would survive. The name assertion covers the other
/// case: if the outer name didn't win the slot, the buffer result would be
/// the same but the recorded change would name the inner `"delete"`.
#[test]
fn steel_dot_repeatable_round_trip() {
    let mut ed = editor_with_steel(
        "-[foo]> bar\n",
        r#"(define-command! "del-sel" ""
             (lambda () (call! "delete" (focused-pane)))
             #:repeatable #t)"#,
    );

    // Run the repeatable Steel command.
    ed.execute_keymap_command("del-sel".into(), Some(1), false);
    // "foo" deleted; buffer is " bar\n".
    assert_eq!(ed.doc().text().to_string(), " bar\n", "first run");

    // the recorded change must name the outer Steel command, not inner "delete".
    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("del-sel"),
        "outer Steel command must win the repeat slot over the inner 'delete'"
    );

    // Select "bar" then press `.`: replay must delete the current selection.
    ed.feed_key(key('w')); // select "bar" (bare, indent kept)
    ed.feed_key(key('.')); // replay "del-sel" → delete "bar"
    assert_eq!(
        ed.doc().text().to_string(),
        " \n",
        "`.` must replay the Steel command and delete the current selection"
    );
}

/// A plain `define-command!` (non-repeatable) must not overwrite
/// the recorded change set by a prior native edit.
///
/// If `meta().repeatable` returned `true` for `SteelBacked`, running the
/// Steel command would stamp the recorded change with its name; the
/// subsequent `.` would replay the Steel command instead of the native delete.
#[test]
fn steel_command_is_not_repeatable() {
    let mut ed = editor_with_steel(
        "-[foo]> bar\n",
        r#"(define-command! "del-sel" "" (lambda () (call! "delete" (focused-pane))))"#,
    );

    // Establish a known repeatable native action first.
    ed.feed_key(key('d')); // delete "foo" → " bar\n"
    assert_eq!(ed.doc().text().to_string(), " bar\n");
    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("delete"),
        "setup: the recorded change must be 'delete' after d"
    );

    // Run the Steel command: it calls (call! "delete" (focused-pane)) internally.
    ed.execute_keymap_command("del-sel".into(), Some(1), false);

    // the recorded change must still be "delete", not "del-sel".
    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("delete"),
        "Steel command must not overwrite the recorded change"
    );
}

/// A plain `define-command!` (non-repeatable) must NOT overwrite
/// the recorded change set by a prior native edit.
///
/// If `meta().repeatable` were `true` for every `SteelBacked` command, the
/// non-repeatable Steel command would overwrite the recorded change and
/// the subsequent `.` would replay it.
#[test]
fn non_repeatable_steel_does_not_hijack_dot() {
    let mut ed = editor_with_steel(
        "-[foo]> bar\n",
        r#"(define-command! "noop-move" "" (lambda () (call! "move-right" (focused-pane))))"#,
    );

    // Establish a known repeatable native action first.
    ed.feed_key(key('d')); // delete "foo" → " bar\n"
    assert_eq!(ed.doc().text().to_string(), " bar\n");
    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("delete"),
        "setup: the recorded change must be 'delete' after d"
    );

    // Run the non-repeatable Steel command.
    ed.execute_keymap_command("noop-move".into(), Some(1), false);

    // the recorded change must still be "delete".
    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("delete"),
        "non-repeatable Steel command must not overwrite the recorded change"
    );

    // `.` must still delete "bar".
    ed.feed_key(key('w'));
    ed.feed_key(key('.'));
    assert!(
        !ed.doc().text().to_string().contains("bar"),
        "`.` must repeat the native delete, not the non-repeatable Steel command"
    );
}

// ── Insert-key dot-repeat (keymap-bound native commands, `insert-key!`) ──────

/// `Ctrl-w` deletes a word mid-Insert-session, dispatched as an Insert-key
/// binding: `.` on a different selection must repeat that delete too, not
/// just the raw keys typed around it.
///
/// Both edit sites sit right after a `(`, a non-word char, so
/// `delete-word-backward`'s word-boundary scan stops there regardless of
/// what came before, and the two sites are directly comparable. (Landing
/// mid-word instead, as `c` on a bare selection with existing text right
/// before the cursor would, makes the two sites' word-boundary context
/// differ, since the replayed delete would then span back into that
/// pre-existing text too, which is `delete-word-backward` working as
/// designed, not a replay bug; `set_current_selections` below sidesteps
/// that entirely by selecting the second target directly, rather than via
/// a word motion whose landing spot relative to the punctuation would add
/// another such context difference.)
///
/// "hello" typed then deleted word-backward nets to
/// nothing, so a correct replay gives the exact same result as typing just
/// "hi" directly into either pair of parens. If `Ctrl-w` weren't replayed,
/// the typed "hello" would survive in the second pair: "(hi) (hellohi)\n".
#[test]
fn dot_repeats_ctrl_w_inside_insert() {
    let mut ed = editor_from("(-[foo]>) (bar)\n");

    ed.feed_key(key('c')); // change: delete "foo", enter Insert; cursor is now right after "("
    ed.feed_key(key('h'));
    ed.feed_key(key('e'));
    ed.feed_key(key('l'));
    ed.feed_key(key('l'));
    ed.feed_key(key('o')); // "(hello) (bar)\n"
    ed.feed_key(key_ctrl('w')); // delete "hello" word-backward, back to "("
    ed.feed_key(key('h'));
    ed.feed_key(key('i'));
    ed.feed_key(key_esc()); // back to Normal

    assert_eq!(ed.doc().text().to_string(), "(hi) (bar)\n");

    select_marked(&mut ed, "(hi) (-[bar]>)\n");
    ed.feed_key(key('.')); // repeat: delete "bar", type "hello", Ctrl-w, type "hi"

    assert_eq!(ed.doc().text().to_string(), "(hi) (hi)\n");
}

/// Same proof for a Steel command that calls a native command mid-Insert
/// (`dispatch`'s Steel path → `call!` → `run_command_sync`), rather than a
/// native binding like `Ctrl-w`. The expected result is derived the same way
/// as in `dot_repeats_ctrl_w_inside_insert`.
#[test]
fn steel_insert_binding_calling_native_command_is_recorded_for_dot_repeat() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("(-[foo]>) (bar)\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "del-word" "" (lambda (pane) (call! "delete-word-backward" pane)))
           (bind-key! 'insert "ctrl-x" "del-word")"#,
    );

    ed.feed_key(key('c'));
    ed.feed_key(key('h'));
    ed.feed_key(key('e'));
    ed.feed_key(key('l'));
    ed.feed_key(key('l'));
    ed.feed_key(key('o'));
    ed.feed_key(key_ctrl('x')); // Steel wrapper → (call! "delete-word-backward" pane)
    ed.feed_key(key('h'));
    ed.feed_key(key('i'));
    ed.feed_key(key_esc());

    assert_eq!(ed.doc().text().to_string(), "(hi) (bar)\n");

    select_marked(&mut ed, "(hi) (-[bar]>)\n");
    ed.feed_key(key('.'));

    assert_eq!(ed.doc().text().to_string(), "(hi) (hi)\n");
}

/// `.` after a Tab binding took its "insert a tab" branch (via
/// `insert-key!`) repeats exactly one tab, proving the binding itself is
/// recorded as a `Command`, re-run once on replay (not the `insert-key!`
/// fallback's raw key; see `Input`'s own doc for why the binding is
/// what's recorded), and recorded only once.
///
/// Identical setup and expected result to `tabs.rs`'s
/// `dot_repeat_replays_tab` (the plain unbound-Tab case): `tab-or-complete`
/// must reproduce that exact behaviour, not a doubled or dropped tab.
#[test]
fn dot_repeat_replays_tab_via_insert_key_once() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "tab-or-complete" "" (lambda (pane) (insert-key! pane "tab")))
           (bind-key! 'insert "tab" "tab-or-complete")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key_tab());
    ed.feed_key(key_esc()); // Esc selects the typed run, just the tab
    assert_eq!(state(&ed), "-[\t]>hello\n");

    ed.feed_key(key('l'));
    ed.feed_key(key('l'));
    ed.feed_key(key('.'));
    assert_eq!(state(&ed), "\th-[\t]>ello\n");

    let action = ed
        .state
        .last_action()
        .expect("`i` must have stamped a repeatable action");
    let inputs = &action.transcript;
    assert_eq!(
        inputs.len(),
        2,
        "the tab-or-complete binding once, not doubled, then Esc: {inputs:?}"
    );
    assert!(
        is_binding(&inputs[0], "tab-or-complete") && is_binding(&inputs[1], "exit-insert"),
        "must record the binding, not a raw Key: {inputs:?}"
    );
}

/// A binding that leaves Insert and re-enters it keeps recording into the
/// same repeatable action: `.` must not lose everything typed before the
/// binding ran, nor what was typed after it.
#[test]
fn insert_key_binding_reentering_insert_keeps_earlier_keys() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "reenter-tab" "" (lambda (pane)
             (call! "exit-insert" pane)
             (call! "insert-at-line-end" pane)
             (insert-key! pane "tab")))
           (bind-key! 'insert "ctrl-x" "reenter-tab")"#,
    );

    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    ed.feed_key(key('b'));
    ed.feed_key(key_ctrl('x'));
    ed.feed_key(key('c'));
    ed.feed_key(key('d'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "abhello\tcd\nworld\n");

    let action = ed
        .state
        .last_action()
        .expect("`i` must have stamped a repeatable action");
    let keys = &action.transcript;
    assert_eq!(keys.len(), 6, "got {keys:?}");
    assert!(
        is_key(&keys[0], 'a') && is_key(&keys[1], 'b'),
        "got {keys:?}"
    );
    assert!(is_binding(&keys[2], "reenter-tab"), "got {keys:?}");
    assert!(
        is_key(&keys[3], 'c') && is_key(&keys[4], 'd'),
        "got {keys:?}"
    );
    assert!(is_binding(&keys[5], "exit-insert"), "got {keys:?}");

    ed.feed_key(key('j'));
    ed.feed_key(key('g'));
    ed.feed_key(key('h'));
    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "abhello\tcd\nabworld\tcd\n");
}

/// Records `i`, `a`, a `ctrl-x` binding running `body`, Esc on
/// `"hello\nworld\n"`, then moves to the start of line 2, the shared
/// setup for the replay-failure tests below.
fn record_insert_key_binding_then_move(body: &str) -> (Editor, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\nworld\n");
    run(
        &mut ed,
        tmp.path(),
        &format!(
            r#"{body}
               (bind-key! 'insert "ctrl-x" "tab-key")"#
        ),
    );
    ed.feed_key(key('i'));
    ed.feed_key(key('a'));
    ed.feed_key(key_ctrl('x'));
    ed.feed_key(key_esc());
    assert_eq!(ed.doc().text().to_string(), "a\thello\nworld\n");
    ed.feed_key(key('j'));
    ed.feed_key(key('g'));
    ed.feed_key(key('h'));
    (ed, tmp)
}

/// A recorded `Binding` whose command was unregistered before `.` (a plugin
/// reload) reports and is skipped. Replay continues to the next recorded
/// entry (`exit-insert`), which is what actually closes the session, and
/// the repeatable action itself is left intact.
#[test]
fn dot_repeat_reports_an_unregistered_insert_key_binding_and_skips_it() {
    let (mut ed, _tmp) = record_insert_key_binding_then_move(
        r#"(define-command! "tab-key" "" (lambda (pane) (insert-key! pane "tab")))"#,
    );
    ed.state.config.registry.unregister("tab-key");

    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "a\thello\naworld\n");
    assert!(
        status(&ed).contains("unknown command"),
        "got: {}",
        status(&ed)
    );
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("insert-at-selection-start")
    );
}

/// A recorded `Binding` whose Steel body fails on replay (it succeeded the
/// first time) is skipped the same way.
#[test]
fn dot_repeat_skips_an_insert_key_binding_that_fails_on_replay() {
    let (mut ed, _tmp) = record_insert_key_binding_then_move(
        r#"(define tab-key-runs 0)
           (define-command! "tab-key" "" (lambda (pane)
             (set! tab-key-runs (+ tab-key-runs 1))
             (if (> tab-key-runs 1) (error "boom") (insert-key! pane "tab"))))"#,
    );

    ed.feed_key(key('.'));
    assert_eq!(ed.doc().text().to_string(), "a\thello\naworld\n");
    assert!(status(&ed).contains("boom"), "got: {}", status(&ed));
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert_eq!(
        ed.state.last_action().map(|a| a.command.as_ref()),
        Some("insert-at-selection-start")
    );
}

/// An unbound Insert key with no default behaviour (an unbound Ctrl-chord)
/// does nothing, so it must not be recorded for dot-repeat. Only the typed
/// char that follows it, and the Esc binding, are.
#[test]
fn unbound_insert_key_with_no_default_behaviour_is_not_recorded() {
    let mut ed = editor_from("-[h]>ello\n");

    ed.feed_key(key('i'));
    ed.feed_key(key_ctrl('x'));
    ed.feed_key(key('z'));
    ed.feed_key(key_esc());
    assert_eq!(state(&ed), "-[z]>hello\n");

    let action = ed
        .state
        .last_action()
        .expect("`i` must have stamped a repeatable action");
    let inputs = &action.transcript;
    assert_eq!(inputs.len(), 2, "got {inputs:?}");
    assert!(
        is_key(&inputs[0], 'z') && is_binding(&inputs[1], "exit-insert"),
        "only the typed `z` and Esc may be recorded, got {inputs:?}"
    );
}

/// A Tab binding that takes its "trigger completion" branch marks itself
/// interactive and writes nothing itself, so its `Binding` placeholder is
/// dropped rather than recorded: `.` never reopens the popup at all.
#[test]
fn tab_or_complete_completion_branch_leaves_no_popup_after_dot_repeat() {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("-[h]>ello\n");
    let source = format!(
        "{}\n{}",
        completion_source("test", &completion_labels(&["foo"]), ""),
        r#"(define-command! "tab-or-complete" "" (lambda (pane) (call! "completion-trigger" pane)))
           (bind-key! 'insert "tab" "tab-or-complete")"#
    );
    run(&mut ed, tmp.path(), &source);

    ed.feed_key(key('i'));
    ed.feed_key(key_tab());
    ed.settle();
    ed.feed_key(key_esc()); // dismisses the popup, staying in Insert
    ed.feed_key(key_esc()); // leaves Insert

    let inputs = &ed
        .state
        .last_action()
        .expect("`i` must have stamped a repeatable action")
        .transcript;
    assert_eq!(inputs.len(), 1, "got {inputs:?}");
    assert!(is_binding(&inputs[0], "exit-insert"), "got {inputs:?}");

    ed.feed_key(key('.'));
    ed.settle();
    assert_eq!(ed.state.mode(), Mode::Normal);
    assert!(ed.state.input.buffer_completion().is_none());
    assert_eq!(ed.doc().text().to_string(), "hello\n");
}

/// Drives `c`, types "he", opens completion, then presses `accept`, which
/// must accept "hello", optionally followed by `then`, and leaves Insert.
/// Whether `accept` is the popup's own Enter or an Insert-key binding
/// calling `completion-accept!`, the result is recorded the same way: one
/// `Input::Result` covering the whole accept, and, for a binding that
/// edits further after accepting, that follow-up edit too. The two can't
/// be split, since neither is safe to re-derive once either has gone
/// interactive (see `DotCapture`'s own doc). The `completion-trigger`
/// dispatch (Ctrl-Space) that opened the popup marks itself interactive too
/// and writes nothing itself, so its own placeholder never appears at all.
/// Asserts that shape, and that `.` on "bar" reproduces the same text in one
/// undo step.
///
/// The text a live `c he<accept>` wrote in the first
/// pair of parens is exactly what `.` must write in the second.
fn assert_accepted_completion_is_dot_repeated(extra_script: &str, accept: KeyEvent, then: &str) {
    let tmp = tempfile::tempdir().unwrap();
    let mut ed = editor_from("(-[foo]>) (bar)\n");
    let source = format!(
        "{}\n{extra_script}",
        completion_source("test", &completion_labels(&["hello"]), "")
    );
    run(&mut ed, tmp.path(), &source);

    ed.feed_key(key('c'));
    ed.feed_key(key('h'));
    ed.feed_key(key('e'));
    ed.feed_key(key_ctrl(' '));
    ed.settle();
    ed.feed_key(accept);
    ed.feed_key(key_esc());
    assert_eq!(
        ed.doc().text().to_string(),
        format!("(hello{then}) (bar)\n")
    );

    let inputs = &ed
        .state
        .last_action()
        .expect("`c` must have stamped a repeatable action")
        .transcript;
    assert_eq!(inputs.len(), 4, "got {inputs:?}");
    let expect_text = format!("hello{then}");
    assert!(
        is_key(&inputs[0], 'h')
            && is_key(&inputs[1], 'e')
            && matches!(&inputs[2], Input::Result(r)
                if r.back == 2 && r.forward == 0 && r.text == expect_text)
            && is_binding(&inputs[3], "exit-insert"),
        "got {inputs:?}"
    );

    select_marked(&mut ed, &format!("(hello{then}) (-[bar]>)\n"));
    ed.feed_key(key('.'));
    assert_eq!(
        ed.doc().text().to_string(),
        format!("(hello{then}) (hello{then})\n")
    );

    ed.feed_key(key('u'));
    assert_eq!(
        ed.doc().text().to_string(),
        format!("(hello{then}) (bar)\n")
    );
}

#[test]
fn dot_repeats_a_completion_accepted_with_enter() {
    assert_accepted_completion_is_dot_repeated("", key_enter(), "");
}

/// An Insert-key binding calling `completion-accept!` is never re-run by
/// `.`: its net edit is recorded as an `Input::Result` and replayed
/// directly, with no completion session to pick from (see that variant's
/// own doc).
#[test]
fn dot_repeats_a_completion_accepted_by_an_insert_key_binding() {
    assert_accepted_completion_is_dot_repeated(
        r#"(define-command! "accept-first" "" (lambda (pane) (completion-accept! 0)))
           (bind-key! 'insert "ctrl-y" "accept-first")"#,
        key_ctrl('y'),
        "",
    );
}

/// A binding that accepts and then edits records the whole dispatch as one
/// net edit (the accepted text and the follow-up edit together, not the
/// accept alone), and `.` replays that one edit, never the binding itself.
#[test]
fn dot_repeats_an_insert_key_binding_that_accepts_then_edits() {
    assert_accepted_completion_is_dot_repeated(
        r#"(define-command! "accept-space" "" (lambda (pane)
             (completion-accept! 0)
             (insert-key! pane "space")))
           (bind-key! 'insert "ctrl-y" "accept-space")"#,
        key_ctrl('y'),
        " ",
    );
}
