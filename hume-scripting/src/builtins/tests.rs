// These two tests exercise a `cmd` and a `config` table entry through a
// real `ScriptingHost` (register_all → Steel dispatch → the wrapper
// closure's gate call) rather than calling `errors::require_cmd`/
// `require_config` directly — proving the registration table's kind tags
// actually wire a builtin to its gate, which the per-builtin unit tests
// (calling the gate primitive with the builtin's name as a string) can't
// catch on their own: a `cmd` entry mistyped as `open` would silently
// stop gating without failing any of those.

/// A `cmd`-gated builtin (`current-buffer`) called from init.scm — where
/// `register_all`'s wrapper closure is the only place the gate lives —
/// must still raise "not available during init".
#[test]
fn cmd_gated_builtin_rejected_from_init_through_real_registration() {
    let mut host = crate::ScriptingHost::new();
    let mut null_host = crate::null_host::NullHost;
    let err = host
        .eval_source("(current-buffer)", &mut null_host)
        .expect_err("current-buffer must be rejected during init.scm eval");
    assert!(err.contains("not available during init"), "got: {err}");
}

/// A `config`-gated builtin (`bind-key!`) called from inside a command body
/// (`EvalMode::Command`, dispatched via the real `call_steel_cmd` path) must
/// still raise "not from a Steel command body".
///
/// `set-option!` used to be this test's example builtin, but it's `open`
/// now (callable from any context — see `builtins/settings.rs`'s doc);
/// `bind-key!` remains genuinely `config`-gated.
#[test]
fn config_gated_builtin_rejected_from_command_body_through_real_registration() {
    let mut host = crate::ScriptingHost::new();
    let mut null_host = crate::null_host::NullHost;
    host.eval_source(
        r#"(define-command! "probe-bind-key" "doc" (lambda () (bind-key! 'normal "Q" "move-down")))"#,
        &mut null_host,
    )
    .expect("defining the probe command must not error");

    let err = host
        .call_steel_cmd(
            "probe-bind-key",
            None,
            vec![],
            hume_engine::pipeline::PaneId::default(),
            hume_engine::pipeline::BufferId::default(),
            &mut null_host,
        )
        .expect_err("bind-key! must be rejected from a command body");
    assert!(err.message.contains("command body"), "got: {err:?}");
}
