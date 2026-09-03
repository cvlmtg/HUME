//! Tests for the consolidated `hume_scripting::Effect` log: the
//! platform-neutral atomic-eval contract. The plugin-loading tests (Scheme
//! require strings embed OS paths) live in `unix/scripting_effects.rs`.

use super::*;
use crate::editor::scripting_setup::make_init_host;
use hume_scripting::ScriptingHost;

/// Atomic-eval contract, exercised through `call_steel_cmd` (a plain
/// command dispatch, no nested plugin activation involved — that path has
/// its own independent rollback via `pop_effect_marks`, already covered by
/// `hume-scripting`'s `queued_effects_before_failure_are_rolled_back`). A
/// command body that queues a `register-lsp-server!` effect and then errors
/// must leave nothing behind in the log — with no nested activation to
/// commit anything, `ScriptingHost::take_eval_effects` has nothing to
/// salvage on `Err`, so the whole eval's own uncommitted entries are dropped.
///
/// Flip: in `take_eval_effects`, salvage every entry regardless of
/// `committed` on the `Err` arm — this test starts failing because
/// `host.effects_for_test()` comes back non-empty (the queued
/// `register-lsp-server!` survives the error).
#[test]
fn failed_command_eval_effects_do_not_leak() {
    let mut ed = editor_from("-[a]>bcdef\n");
    let mut host = ScriptingHost::new();
    {
        let mut ih = make_init_host(
            &mut ed.state,
            &mut ed.view,
            ed.terminal.as_ref(),
            ed.tui_active,
            ed.kitty_enabled,
        );
        host.eval_source(
            r#"(define-command! "efx-fail" ""
                 (lambda ()
                   (register-lsp-server! "leaked" #:command "leaked-lsp")
                   (error "intentional mid-body failure")))"#,
            &mut ih,
        )
    }
    .expect("define-command! must succeed");

    let pid = ed.state.focused_pane_id;
    let bid = ed.focused_buffer_id();
    let result = {
        let mut ih = make_init_host(
            &mut ed.state,
            &mut ed.view,
            ed.terminal.as_ref(),
            ed.tui_active,
            ed.kitty_enabled,
        );
        host.call_steel_cmd("efx-fail", None, vec![], pid, bid, &mut ih)
    };
    assert!(
        result.is_err(),
        "the command must fail on its intentional error"
    );
    assert!(
        host.effects_for_test().is_empty(),
        "the failed command's queued register-lsp-server! effect must not survive; got: {:?}",
        host.effects_for_test()
    );
}
