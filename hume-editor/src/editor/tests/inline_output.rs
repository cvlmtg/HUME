//! `#:inline-output` reached via `(call! …)` — from a key-bound command's own
//! body, a hook, or a queued timer thunk — rather than dispatched directly
//! by keypress or `:`. See `Editor::call_steel_command_body`'s doc for the
//! direct-dispatch case this complements; `dispatch.rs`'s own
//! `inline_output_commands_own_warning_does_not_shadow_its_own_reload_confirm`
//! (`disk_change.rs`) and the `unix::injections_editor` bracket tests cover
//! that side.
//!
//! Every probe here calls `(%stdout-gate!)` directly and logs which branch it
//! took — the same builtin every gated print shim (`displayln`, …) calls
//! before writing, so a probe result is exactly what a real print would have
//! done, without needing a real terminal.

use super::*;
use crate::editor::InlineOutputDispatch;
use crate::editor::keymap::BindMode;
use hume_scripting::ScriptingHost;

/// Bind `cmd` to `\` in Normal mode — the one key every test here uses, so a
/// bound command's body can be reached with a single `feed_event`.
fn bind_backslash(ed: &mut Editor, cmd: &str) {
    ed.state.config.keymap.bind_user_with_extend(
        BindMode::Normal,
        &[key('\\')],
        cmd.to_owned().into(),
        false,
    );
}

fn logged(ed: &Editor, needle: &str) -> bool {
    ed.state
        .message_log
        .entries()
        .any(|e| e.text.contains(needle))
}

// ── call! reaches a declared command ────────────────────────────────────────

/// A plain (non-`#:inline-output`) editor command's own body can `call!` an
/// `#:inline-output` command and have *that* command's prints reach the
/// gate — the runtime gap `%dispatch-command`'s in-VM `(apply proc args)`
/// otherwise left open (only keypress/`:` dispatch armed the bracket before
/// this).
///
/// Fail oracle: drop `(%arm-inline-output! name)` from `%apply-command`
/// (`bootstrap.scm`) → logs `"gate-closed"`.
#[test]
fn call_bang_to_inline_output_editor_command_opens_the_gate() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "inner-probe" ""
             (lambda () (log! 'warn (if (%stdout-gate!) "gate-open" "gate-closed")))
             #:inline-output #t)
           (define-command! "trigger" "" (lambda () (call! "inner-probe")))"#,
    );
    bind_backslash(&mut ed, "trigger");

    ed.feed_event(key('\\'));

    assert!(
        logged(&ed, "gate-open"),
        "call! to an #:inline-output command must open the print gate; messages: {:?}",
        ed.state.message_log.entries().collect::<Vec<_>>()
    );
}

/// A typed command is never reachable through `call!` — `%dispatch-command`
/// only ever resolves `command_table` (mappable names), never
/// `typed_command_table` — so declaring one `#:inline-output` gives it no
/// `call!` path to arm through. Pins the separation rather than assuming it:
/// if it ever broke, `inner-typed-probe`'s body would silently start
/// running from a `call!` site nothing should reach it from.
#[test]
fn call_bang_to_inline_output_typed_command_is_unreachable() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-typed-command! "inner-typed-probe" ""
             (lambda () (log! 'warn "should-not-run"))
             #:inline-output #t)
           (define-command! "trigger" "" (lambda () (call! "inner-typed-probe")))"#,
    );
    bind_backslash(&mut ed, "trigger");

    ed.feed_event(key('\\'));

    assert!(
        !logged(&ed, "should-not-run"),
        "a typed command's body must never run via call!"
    );
    assert!(
        logged(&ed, "unknown command"),
        "call! to a typed-only name must fall through as unresolvable — it's absent \
         from the mappable registry `command_is_native` consults, same as any other \
         name never registered as a mappable command"
    );
    assert!(
        !ed.inline_output_entered(),
        "nothing armed, so the bracket must never have been entered"
    );
}

// ── A non-declared call! never touches the bracket ─────────────────────────

/// `call!` to a command that is *not* declared `#:inline-output` must never
/// arm the bracket, even mid-body of an unrelated command and even with a
/// live TUI.
///
/// Fail oracle: drop the declared-flag check from
/// `EditorHostImpl::arm_inline_output` (arm unconditionally) → `plain-inner`
/// observes an open gate it never declared.
#[test]
fn call_bang_to_a_non_declared_command_never_arms() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "plain-inner" ""
             (lambda () (log! 'warn (if (%stdout-gate!) "plain-inner-open" "plain-inner-closed"))))
           (define-command! "trigger" "" (lambda () (call! "plain-inner")))"#,
    );
    bind_backslash(&mut ed, "trigger");

    ed.feed_event(key('\\'));

    assert!(
        logged(&ed, "plain-inner-closed"),
        "a call! to a non-declared command must never open the gate"
    );
    assert!(!logged(&ed, "plain-inner-open"));
}

// ── Nesting ──────────────────────────────────────────────────────────────────

/// An `#:inline-output` command already dispatched (and already past its
/// first print, so the alt-screen is `Entered`) that `call!`s a *second*
/// `#:inline-output` command mid-body must have its own remaining prints
/// still reach the gate once the nested call returns — the nested arm must
/// save and restore the outer's state, not stomp it.
///
/// Fail oracle: make `restore_inline_output` reset unconditionally to
/// `Inactive` instead of popping the saved value → `"outer-after-closed"`
/// logs instead of `"outer-after-open"`.
#[test]
fn nested_call_bang_restores_the_outer_commands_state() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "nested-inner" "" (lambda () (log! 'warn "inner-ran")) #:inline-output #t)
           (define-command! "nested-outer" ""
             (lambda ()
               (log! 'warn (if (%stdout-gate!) "outer-before-open" "outer-before-closed"))
               (call! "nested-inner")
               (log! 'warn (if (%stdout-gate!) "outer-after-open" "outer-after-closed")))
             #:inline-output #t)"#,
    );
    bind_backslash(&mut ed, "nested-outer");

    ed.feed_event(key('\\'));

    assert!(
        logged(&ed, "outer-before-open"),
        "sanity: outer must open its own bracket first"
    );
    assert!(
        logged(&ed, "inner-ran"),
        "sanity: the nested call must have actually run"
    );
    assert!(
        logged(&ed, "outer-after-open"),
        "outer's bracket must still be open after the nested call! returns; messages: {:?}",
        ed.state.message_log.entries().collect::<Vec<_>>()
    );
    assert!(!logged(&ed, "outer-after-closed"));
}

// ── Error unwind ─────────────────────────────────────────────────────────────

/// A body that raises between `%arm-inline-output!` and
/// `%restore-inline-output!` leaves `inline_output_saved` un-popped — the
/// backstop is `run_steel_session`'s own unconditional drain at the tail of
/// the session, not a Steel-side unwind (Steel has no unwind-safe hook to
/// pair with here — nesting `with-handler` + a re-raised native error is the
/// pinned VM-stack-corruption hazard).
///
/// Fail oracle: remove the `ctx.host.output()...reset_inline_output()` call
/// from `run_steel_session` (`activation.rs`) → `inline_output_saved` stays
/// non-empty after the raising dispatch returns.
#[test]
fn raise_inside_call_bang_to_inline_output_command_does_not_leak_saved_state() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "raiser" "" (lambda () (error "boom")) #:inline-output #t)
           (define-command! "outer-raises" "" (lambda () (call! "raiser")) #:inline-output #t)"#,
    );
    bind_backslash(&mut ed, "outer-raises");

    ed.feed_event(key('\\'));

    assert!(
        ed.state.inline_output_saved.is_empty(),
        "a raise between arm and restore must not leave a stale saved entry: {:?}",
        ed.state.inline_output_saved
    );
    assert!(matches!(
        ed.state.inline_output,
        InlineOutputDispatch::Inactive
    ));
}

// ── Hook / timer paths ────────────────────────────────────────────────────────

/// A timer thunk's own `call!` to an `#:inline-output` command must reach
/// the gate too, and leave the bracket closed once the queued-call batch
/// finishes — `run_call_batch` closes it the same way
/// `call_steel_command_body` does for direct dispatch.
///
/// Fail oracle: drop `self.close_inline_output_bracket()` from
/// `run_call_batch` (`scripting_setup.rs`) → this test still passes (the
/// gate opens either way) but a later dispatch would inherit a stuck bracket
/// — covered instead by asserting the reset here, immediately after the
/// batch that armed it.
#[test]
fn timer_call_bang_to_inline_output_command_opens_the_gate() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    let mut host = ScriptingHost::new();
    eval_with_real_host(
        &mut ed,
        &mut host,
        r#"(define-command! "inner-probe" ""
             (lambda () (log! 'warn (if (%stdout-gate!) "gate-open" "gate-closed")))
             #:inline-output #t)
           (define-typed-command! "start" "" (lambda () (after 0 (lambda () (call! "inner-probe")))))"#,
        tmp.path(),
    );
    ed.scripting = Some(host);

    type_cmd(&mut ed, ":start");
    ed.drain_async_sources();
    ed.settle();

    assert!(
        logged(&ed, "gate-open"),
        "a timer thunk's call! to an #:inline-output command must reach the print gate"
    );
    assert!(matches!(
        ed.state.inline_output,
        InlineOutputDispatch::Inactive
    ));
}
