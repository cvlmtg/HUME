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
use crate::editor::keymap::BindMode;
use crate::editor::tui::Tui;

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

/// Build an editor over `"-[a]>bcdef\n"`, define `source`'s commands, bind
/// `cmd` to `\` in Normal mode, and dispatch it — the setup+dispatch shape
/// every `call!`-nesting test below needs, differing only in `source`/`cmd`,
/// whether the dispatch runs with a live TUI, and what each asserts
/// afterward.
fn dispatch_backslash(source: &str, cmd: &str, with_live_tui: bool) -> Editor {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    ed.tui = if with_live_tui {
        Tui::OnHeadless
    } else {
        Tui::Off
    };
    run(&mut ed, tmp.path(), source);
    bind_backslash(&mut ed, cmd);
    ed.feed_event(key('\\'));
    ed
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
///
/// Also covers the depth-`0` restore: `trigger` itself is not declared
/// `#:inline-output`, so `%arm-inline-output!` returns `#f` for it and its
/// own gate reads closed both before and after the nested `call!` — the
/// common case, distinct from `nested_call_bang_restores_the_outer_commands_state`
/// below, where the *outer* command is itself declared and restores to depth
/// `1`. `%apply-command` discriminates "no restore" from "restore to this
/// depth" with `(when depth …)`, which only works because Steel's `is_truthy`
/// treats `(IntV 0)` as true — pinning the after-open/after-closed pair here
/// exercises that depth-`0` branch instead of assuming it.
///
/// Fail oracle: make `%apply-command` skip the restore whenever `depth` is
/// `0` (a plausible but wrong "falsy" reading) → logs `"trigger-after-open"`
/// instead of `"trigger-after-closed"`.
#[test]
fn call_bang_to_inline_output_editor_command_opens_the_gate() {
    let ed = dispatch_backslash(
        r#"(define-command! "inner-probe" ""
             (lambda () (log! 'warn (if (%stdout-gate!) "gate-open" "gate-closed")))
             #:inline-output #t)
           (define-command! "trigger" ""
             (lambda ()
               (call! "inner-probe")
               (log! 'warn (if (%stdout-gate!) "trigger-after-open" "trigger-after-closed"))))"#,
        "trigger",
        false,
    );

    assert!(
        logged(&ed, "gate-open"),
        "call! to an #:inline-output command must open the print gate; messages: {:?}",
        ed.state.message_log.entries().collect::<Vec<_>>()
    );
    assert!(
        logged(&ed, "trigger-after-closed"),
        "trigger's own gate must be closed again once inner-probe's call! returns; \
         messages: {:?}",
        ed.state.message_log.entries().collect::<Vec<_>>()
    );
    assert!(!logged(&ed, "trigger-after-open"));
}

/// A typed command is never reachable through `call!` — `%dispatch-command`
/// only ever resolves `command_table` (mappable names), never
/// `typed_command_table` — so declaring one `#:inline-output` gives it no
/// `call!` path to arm through. Pins the separation rather than assuming it:
/// if it ever broke, `inner-typed-probe`'s body would silently start
/// running from a `call!` site nothing should reach it from.
#[test]
fn call_bang_to_inline_output_typed_command_is_unreachable() {
    let ed = dispatch_backslash(
        r#"(define-typed-command! "inner-typed-probe" ""
             (lambda () (log! 'warn "should-not-run"))
             #:inline-output #t)
           (define-command! "trigger" "" (lambda () (call! "inner-typed-probe")))"#,
        "trigger",
        false,
    );

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
    assert_eq!(
        ed.inline_output_enter_count(),
        0,
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
    let ed = dispatch_backslash(
        r#"(define-command! "plain-inner" ""
             (lambda () (log! 'warn (if (%stdout-gate!) "plain-inner-open" "plain-inner-closed"))))
           (define-command! "trigger" "" (lambda () (call! "plain-inner")))"#,
        "trigger",
        false,
    );

    assert!(
        logged(&ed, "plain-inner-closed"),
        "a call! to a non-declared command must never open the gate"
    );
    assert!(!logged(&ed, "plain-inner-open"));
}

// ── Nesting ──────────────────────────────────────────────────────────────────

/// An `#:inline-output` command already dispatched (and already past its
/// first print, so the alt-screen has already been entered) that `call!`s a
/// *second* `#:inline-output` command mid-body must have its own remaining
/// prints still reach the gate once the nested call returns — the nested
/// arm must truncate back to its own saved depth, not wipe the whole stack.
///
/// Fail oracle: make `%restore-inline-output!` always truncate to `0`
/// instead of the depth `%arm-inline-output!` returned → `"outer-after-closed"`
/// logs instead of `"outer-after-open"`.
#[test]
fn nested_call_bang_restores_the_outer_commands_state() {
    let ed = dispatch_backslash(
        r#"(define-command! "nested-inner" "" (lambda () (log! 'warn "inner-ran")) #:inline-output #t)
           (define-command! "nested-outer" ""
             (lambda ()
               (log! 'warn (if (%stdout-gate!) "outer-before-open" "outer-before-closed"))
               (call! "nested-inner")
               (log! 'warn (if (%stdout-gate!) "outer-after-open" "outer-after-closed")))
             #:inline-output #t)"#,
        "nested-outer",
        false,
    );

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
/// `%restore-inline-output!` leaves its frame untruncated — the backstop is
/// `run_steel_session`'s own unconditional truncate-to-zero at the tail of
/// the session, not a Steel-side unwind (Steel has no unwind-safe hook to
/// pair with here — nesting `with-handler` + a re-raised native error is
/// the pinned VM-stack-corruption hazard).
///
/// Fail oracle: remove the `ctx.host.output()...truncate_inline_output(0)`
/// call from `run_steel_session` (`activation.rs`) → `is_open()` stays
/// `true` after the raising dispatch returns.
#[test]
fn raise_inside_call_bang_to_inline_output_command_does_not_leak_saved_state() {
    let ed = dispatch_backslash(
        r#"(define-command! "raiser" "" (lambda () (error "boom")) #:inline-output #t)
           (define-command! "outer-raises" "" (lambda () (call! "raiser")) #:inline-output #t)"#,
        "outer-raises",
        false,
    );

    assert!(
        !ed.state.inline_output.is_open(),
        "a raise between arm and restore must not leave a stale frame"
    );
}

// ── Hook / timer paths ────────────────────────────────────────────────────────

/// A timer thunk's own `call!` to an `#:inline-output` command must reach
/// the gate too, and leave the bracket closed once the queued-call batch
/// finishes — `run_call_batch`'s own `apply_script_result` call closes it the
/// same way `call_steel_command_body` does for direct dispatch (see
/// `apply_script_result`'s own doc).
///
/// Fail oracle: drop `self.close_inline_output_bracket()` from
/// `apply_script_result` (`scripting_setup.rs`) → this test still passes (the
/// gate opens either way) but a later dispatch would inherit a stuck bracket
/// — covered instead by asserting the reset here, immediately after the
/// batch that armed it.
#[test]
fn timer_call_bang_to_inline_output_command_opens_the_gate() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "inner-probe" ""
             (lambda () (log! 'warn (if (%stdout-gate!) "gate-open" "gate-closed")))
             #:inline-output #t)
           (define-typed-command! "start" "" (lambda () (after 0 (lambda () (call! "inner-probe")))))"#,
    );

    type_cmd(&mut ed, ":start");
    ed.drain_async_sources();
    ed.settle();

    assert!(
        logged(&ed, "gate-open"),
        "a timer thunk's call! to an #:inline-output command must reach the print gate"
    );
    assert!(!ed.state.inline_output.is_open());
}

// ── Nesting: the alt-screen enters at most once per dispatch ─────────────────
//
// `Tui::OnHeadless` drives the whole state machine (entering the alt-screen,
// `close_inline_output_bracket`'s physical-teardown branch) without a real
// TTY — see `ActiveTui`'s doc, and `disk_change.rs`'s
// `inline_output_commands_own_warning_does_not_shadow_its_own_reload_confirm`
// for the same pattern.

/// An outer `#:inline-output` command already past its first print (so the
/// alt-screen has already been entered) that `call!`s a second declared
/// command, and then prints again itself after the nested call returns,
/// must still enter the alt-screen exactly once for the whole dispatch.
///
/// Fail oracle: in `InlineOutput::needs_enter`, drop the `entered.is_some()`
/// early return — `enter_count()` reports `2`.
#[test]
fn nested_call_bang_after_entered_does_not_reenter_the_alt_screen() {
    let ed = dispatch_backslash(
        r#"(define-command! "reenter-inner" "" (lambda () (%stdout-gate!)) #:inline-output #t)
           (define-command! "reenter-outer" ""
             (lambda ()
               (%stdout-gate!)
               (call! "reenter-inner")
               (%stdout-gate!))
             #:inline-output #t)"#,
        "reenter-outer",
        true,
    );

    assert_eq!(
        ed.inline_output_enter_count(),
        1,
        "the alt-screen must be entered exactly once across the whole nested dispatch"
    );
    assert!(
        !ed.state.inline_output.is_open(),
        "bracket must be closed once dispatch returns"
    );
}

/// The mirror case: an outer command `call!`s a nested declared command
/// *before* its own first print — the nested call enters the alt-screen
/// first, and the outer's own print (after the nested call returns) must
/// not re-enter it either: the outer's own frame is armed but the alt-screen
/// hasn't been entered yet at the moment it arms the nested frame.
///
/// Fail oracle: same as above — drop the `entered.is_some()` guard.
#[test]
fn nested_call_bang_before_outer_prints_does_not_reenter_the_alt_screen() {
    let ed = dispatch_backslash(
        r#"(define-command! "reenter-inner2" "" (lambda () (%stdout-gate!)) #:inline-output #t)
           (define-command! "reenter-outer2" ""
             (lambda ()
               (call! "reenter-inner2")
               (%stdout-gate!))
             #:inline-output #t)"#,
        "reenter-outer2",
        true,
    );

    assert_eq!(ed.inline_output_enter_count(), 1);
    assert!(!ed.state.inline_output.is_open());
}

// ── Error unwind: a caught (not just an uncaught) raise ──────────────────────

/// A `call!`-armed frame left unpaired by a *caught* Steel error (the raise
/// never reaches dispatch as an `Err`, unlike
/// `raise_inside_call_bang_to_inline_output_command_does_not_leak_saved_state`
/// above) must still be gone, and the bracket still closed, once the whole
/// dispatch returns — the `run_steel_session` tail truncate is the backstop
/// either way, not just for an uncaught raise.
///
/// Fail oracle: drop the `ctx.host.output()...truncate_inline_output(0)`
/// call from `run_steel_session` (`activation.rs`) → `is_open()` stays
/// `true` after this dispatch, and the next dispatch's `SteelCtx` inherits
/// an already-open gate it never armed.
#[test]
fn caught_error_inside_call_bang_still_closes_bracket_and_drains_state() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    ed.tui = Tui::OnHeadless;
    // Drain `editor_with_file`'s own `OnBufferEnter` disk check before the
    // external rewrite below, so it can't produce a `confirm` of its own —
    // see `hook_call_bang_…`'s identical drain.
    ed.settle();
    assert!(ed.state.config.confirm.is_none());
    let scm_dir = safe_tempdir();

    run(
        &mut ed,
        scm_dir.path(),
        r#"(define-command! "raiser2" "" (lambda () (error "boom")) #:inline-output #t)
           (define-command! "outer-catches" ""
             (lambda ()
               (with-handler (lambda (e) (log! 'warn "caught")) (call! "raiser2"))
               (log! 'warn "outer-continued")))"#,
    );
    bind_backslash(&mut ed, "outer-catches");

    // A different length than the fixture's original content, so the
    // on-disk signature differs by size alone regardless of mtime
    // resolution (see `disk_change.rs`'s `rewrite_externally`).
    std::fs::write(&tmp, "hello, world!\n").unwrap();

    ed.feed_event(key('\\'));

    assert!(
        logged(&ed, "outer-continued"),
        "sanity: the outer body must have kept running past the caught raise"
    );
    assert!(
        !ed.state.inline_output.is_open(),
        "the leaked raiser2 frame must not survive the dispatch"
    );
    assert!(
        ed.state.config.confirm.is_some(),
        "raiser2 still ran with the TUI active, so the reload \
         confirm its subprocess caused must still open even though its error \
         was caught"
    );
}

// ── Hook path closes the bracket too ──────────────────────────────────────────

/// A hook body's own `call!` to a declared command must have its bracket
/// closed by `fire_one_event` the same way `run_call_batch` closes it for a
/// timer thunk — both now go through `apply_script_result`'s shared close.
///
/// `is_open()` alone can't tell whether the close actually ran: every Steel
/// session's own tail (`run_steel_session`) drains the frame stack
/// unconditionally regardless (see `caught_error_inside_call_bang_…`'s doc).
/// The oracle here instead is `close_inline_output_bracket`'s *other*
/// job — queuing `OnFocusGained` because the hook ran with the real
/// terminal — which nothing else in this scenario produces: `OnBufferSave`
/// itself triggers no disk check (`scripting_setup.rs`'s event-name match
/// groups it with the events `react_to_event` does nothing extra for), and
/// the initial `settle()` drains `editor_with_file`'s own `OnBufferEnter`
/// disk check before the external rewrite below, so that one can't produce
/// a `confirm` of its own either.
///
/// Fail oracle: drop `self.close_inline_output_bracket()` from
/// `apply_script_result` → `confirm` stays `None`.
#[test]
fn hook_call_bang_to_inline_output_command_closes_the_bracket() {
    let (mut ed, tmp) = editor_with_file("-[h]>ello\n", "hello\n");
    ed.tui = Tui::OnHeadless;
    ed.settle();
    assert!(
        ed.state.config.confirm.is_none(),
        "sanity: nothing has changed on disk yet"
    );

    let scm_dir = safe_tempdir();
    run(
        &mut ed,
        scm_dir.path(),
        r#"(define-command! "hook-inner-probe" "" (lambda () (%stdout-gate!)) #:inline-output #t)
           (register-hook! 'on-buffer-save (lambda (bid) (call! "hook-inner-probe")))"#,
    );

    // A different length than the fixture's original content, so the
    // on-disk signature differs by size alone (see `disk_change.rs`'s
    // `rewrite_externally`).
    std::fs::write(&tmp, "hello, world!\n").unwrap();

    let bid = ed.focused_buffer_id();
    ed.state
        .queue_event(super::super::event::EditorEvent::OnBufferSave { buffer: bid });
    ed.settle();

    assert!(
        ed.state.config.confirm.is_some(),
        "hook-inner-probe ran with the TUI active, so the \
         hook path's close must queue the reload confirm its subprocess \
         caused, the same way direct dispatch's close does"
    );
}

// ── EditorHostImpl::init threads the live terminal state ───────────────────

/// `EditorHostImpl::init` — the constructor behind every init/activation
/// call site, including `Editor::activate_and_register`'s runtime
/// lazy-plugin activation (`mappings/lazy.rs`) — must arm `Tui`-aware
/// rather than hardcoding it, since unlike `init.scm`'s own evals, a runtime
/// activation can run with `Editor::run` already owning the terminal.
///
/// Drives the host `EditorHostImpl::init` builds directly (bypassing the
/// declare/activate plugin ceremony, which is orthogonal to this) with `Tui`
/// both `Off` and active, asserting the alt-screen enters only in the latter
/// case.
///
/// Fail oracle: hardcode `EditorHostImpl::init`'s `tui` parameter to
/// `Tui::Off` → `enter_count` stays `0` after the active block too.
#[test]
fn editor_host_impl_init_threads_live_tui_into_arm() {
    use hume_scripting::host::EditorHost;

    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "init-host-probe" "" (lambda () (%stdout-gate!)) #:inline-output #t)"#,
    );

    // Off the event loop (`Tui::Off`) — the shape `init_scripting`'s own two
    // evals always run in — arms a frame the gate opens for, but
    // `ensure_inline_output_screen` must never enter the (nonexistent)
    // alt-screen for it.
    {
        let mut host = crate::editor::host_impl::EditorHostImpl::init(
            &mut ed.state,
            &mut ed.view,
            Tui::Off,
            false,
        );
        let output = host
            .output()
            .expect("EditorHostImpl always implements OutputHost");
        let depth = output
            .arm_inline_output("init-host-probe")
            .expect("declared #:inline-output");
        output
            .ensure_inline_output_screen()
            .expect("no real terminal to fail against");
        output.truncate_inline_output(depth);
    }
    assert_eq!(
        ed.inline_output_enter_count(),
        0,
        "Tui::Off must never enter the alt-screen"
    );

    // Live `Editor::run` — the shape a runtime lazy plugin activation can be
    // in: a `call!`-armed nested command's raw stdout writes must hit the
    // bracket, not a live alt-screen directly.
    {
        let mut host = crate::editor::host_impl::EditorHostImpl::init(
            &mut ed.state,
            &mut ed.view,
            Tui::OnHeadless,
            false,
        );
        let output = host
            .output()
            .expect("EditorHostImpl always implements OutputHost");
        let depth = output
            .arm_inline_output("init-host-probe")
            .expect("declared #:inline-output");
        output
            .ensure_inline_output_screen()
            .expect("Tui::OnHeadless skips the real I/O");
        output.truncate_inline_output(depth);
    }
    assert_eq!(
        ed.inline_output_enter_count(),
        1,
        "an active Tui must enter the alt-screen exactly once"
    );
}

/// `EditorHostImpl::new` — the convenience constructor for callers with no
/// terminal/`OutputHost` need — must have no inline-output authority at all,
/// regardless of whether `Editor::run`'s loop happens to be live. Unlike
/// `init`'s `Tui::Off`, which legitimately means "not in the loop right now"
/// and still runs the bracket's state machine, `new` has nothing to say about
/// the terminal either way: a frame it can't push is a frame it can't
/// mistakenly enter later against whatever `tui` the *next* host it's rebuilt
/// with happens to carry.
///
/// Fail oracle: give `new` `Some(Tui::Off)` instead of `None` → `declared`
/// still matches, `arm_inline_output` returns `Some(0)` instead of `None`.
#[test]
fn new_host_has_no_inline_output_authority() {
    use hume_scripting::host::EditorHost;

    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "new-host-probe" "" (lambda () (%stdout-gate!)) #:inline-output #t)"#,
    );

    let mut host = crate::editor::host_impl::EditorHostImpl::new(&mut ed.state, &mut ed.view);
    let output = host
        .output()
        .expect("EditorHostImpl always implements OutputHost");
    assert!(
        output.arm_inline_output("new-host-probe").is_none(),
        "a `new` host has no tui to arm a frame against"
    );
    output
        .ensure_inline_output_screen()
        .expect("nothing to enter without an armed frame");
    assert_eq!(
        ed.inline_output_enter_count(),
        0,
        "a `new` host must never enter the alt-screen"
    );
}

/// A frame armed by a real host must be completed correctly by *any* later
/// host that reaches it — including one with no inline-output authority of
/// its own (`EditorHostImpl::new`) — because entry is a property of what the
/// frame itself captured (`Tui::as_active`'s result, and `kitty_enabled`, both
/// at push time), not of whichever host happens to be asking. A guard that
/// instead checks the *asking* host's own `tui`/`kitty_enabled` (as
/// `EditorHostImpl::new`'s lack of authority, and its hardcoded
/// `kitty_enabled: false`, might tempt one to write) would incorrectly skip a
/// frame that really is active, or enter it under the wrong kitty state —
/// this pins the deeper, correct behavior instead.
///
/// Fail oracle (tui): gate `ensure_inline_output_screen` on `self.tui.as_ref()`
/// again (returning early when it's `None`) instead of reading the frame's
/// own captured `tui` from `needs_enter()` → `enter_count` stays `0`.
///
/// Fail oracle (kitty): read `self.kitty_enabled` in
/// `ensure_inline_output_screen` instead of the frame's own captured value →
/// `Entered::kitty` reports `false`, the completing host's own value, instead
/// of `true`, the arming host's.
#[test]
fn a_later_host_with_no_authority_still_completes_a_frame_armed_by_an_earlier_one() {
    use hume_scripting::host::EditorHost;

    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "relay-probe" "" (lambda () (%stdout-gate!)) #:inline-output #t)"#,
    );

    // A real host, with kitty active, arms the frame — the same shape a
    // top-level dispatch or a `call!`-armed nested command would leave
    // behind.
    {
        let mut host = crate::editor::host_impl::EditorHostImpl::init(
            &mut ed.state,
            &mut ed.view,
            Tui::OnHeadless,
            true,
        );
        host.output()
            .expect("EditorHostImpl always implements OutputHost")
            .arm_inline_output("relay-probe")
            .expect("declared #:inline-output");
    }

    // A different host, with no inline-output authority at all and kitty
    // hardcoded `false`, is the one that ends up completing the entry — e.g.
    // a hook fire built its own `EditorHostImpl` between the arm and the
    // first print. The bracket must still open under the kitty state the
    // arming host captured, not this host's own (stale) `kitty_enabled`.
    {
        let mut host = crate::editor::host_impl::EditorHostImpl::new(&mut ed.state, &mut ed.view);
        host.output()
            .expect("EditorHostImpl always implements OutputHost")
            .ensure_inline_output_screen()
            .expect("ActiveTui::Headless skips the real I/O");
    }
    assert_eq!(
        ed.inline_output_enter_count(),
        1,
        "the frame's own captured tui must drive entry, regardless of which \
         host asks"
    );
    assert!(
        ed.state
            .inline_output
            .take_entered()
            .expect("entered")
            .kitty,
        "the frame's own captured kitty must drive entry, not the \
         completing host's own (possibly stale) kitty_enabled"
    );
}

// ── A pushed frame must never outlive a dispatch that never reaches a
//    Steel session ──────────────────────────────────────────────────────────

/// `call_steel_command_body` pushes a frame for a declared `#:inline-output`
/// command before checking whether there is a scripting host to actually run
/// it against. If `self.scripting` is `None` — the registry still knows the
/// command (it was registered before the host went away), but there is
/// nothing left to call — dispatch returns early without ever reaching
/// `run_steel_session` (whose tail truncate-to-zero is what drains every
/// *other* early-exit path) and without calling
/// `close_inline_output_bracket` either. The pushed frame must not survive
/// that early return: a later dispatch's `%stdout-gate!` must not inherit a
/// gate this command never actually opened.
///
/// Fail oracle: this is the bug itself — push happens unconditionally ahead
/// of the `let Some(scripting) = self.scripting.as_mut() else { return false }`
/// guard in `call_steel_command_body`.
#[test]
fn no_scripting_host_does_not_leak_a_pushed_frame() {
    let tmp = safe_tempdir();
    let mut ed = editor_from("-[a]>bcdef\n");
    run(
        &mut ed,
        tmp.path(),
        r#"(define-command! "orphan" "" (lambda () (+ 1 0)) #:inline-output #t)"#,
    );
    bind_backslash(&mut ed, "orphan");
    // Simulate the scripting host having gone away after the command was
    // registered — `run_steel_command` still resolves `orphan` from the
    // registry (a separate store from the host's own `command_table`), so
    // dispatch proceeds all the way to `call_steel_command_body` before
    // finding out there is no host left to call.
    ed.scripting = None;
    ed.feed_event(key('\\'));

    assert!(
        !ed.state.inline_output.is_open(),
        "a dispatch that never reaches a Steel session must not leave a pushed frame behind"
    );
}
