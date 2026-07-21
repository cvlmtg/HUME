use super::*;
use crate::attribution::PluginId;
use crate::test_support::SteelCtxTestHarness;

/// `require_cmd` rejects `Init` and `PluginLoad`, allows the other two.
///
/// Independent oracle: expected pass/fail per state comes from
/// `EvalMode`'s doc table, not from `require_cmd`'s own logic.
#[test]
fn require_cmd_gates_by_mode() {
    let mut h = SteelCtxTestHarness::new();
    assert!(require_cmd(&h.ctx_init(), "x").is_err(), "Init must reject");
    assert!(require_cmd(&h.ctx(), "x").is_ok(), "Command must pass");

    h.plugin_stack
        .push(PluginId::parse("core:test-plugin").unwrap());
    assert!(
        require_cmd(&h.ctx_init(), "x").is_err(),
        "PluginLoad must reject"
    );
    assert!(
        require_cmd(&h.ctx_activation(), "x").is_ok(),
        "PluginActivation must pass"
    );
}

/// `require_config` rejects only `Command`, allows the other three.
#[test]
fn require_config_gates_by_mode() {
    let mut h = SteelCtxTestHarness::new();
    assert!(require_config(&h.ctx_init(), "x").is_ok(), "Init must pass");
    assert!(
        require_config(&h.ctx(), "x").is_err(),
        "Command must reject"
    );

    h.plugin_stack
        .push(PluginId::parse("core:test-plugin").unwrap());
    assert!(
        require_config(&h.ctx_init(), "x").is_ok(),
        "PluginLoad must pass"
    );
    assert!(
        require_config(&h.ctx_activation(), "x").is_ok(),
        "PluginActivation must pass"
    );
}

/// `require_cmd`'s error message names the builtin and mentions "init".
///
/// Fail oracle: drop the `name` interpolation → message no longer
/// identifies which builtin rejected the call.
#[test]
fn require_cmd_error_names_builtin() {
    let mut h = SteelCtxTestHarness::new();
    let err = require_cmd(&h.ctx_init(), "close-buffer!").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("close-buffer!"), "got: {msg}");
    assert!(msg.contains("not available during init"), "got: {msg}");
}

/// `require_config`'s error message names the builtin and mentions
/// "command body".
#[test]
fn require_config_error_names_builtin() {
    let mut h = SteelCtxTestHarness::new();
    let err = require_config(&h.ctx(), "set-option!").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("set-option!"), "got: {msg}");
    assert!(msg.contains("command body"), "got: {msg}");
}

/// A `%`-prefixed registration name (a Rust primitive wrapped by a
/// BOOTSTRAP Scheme function) surfaces in the gate message WITHOUT the
/// `%` — the message must name the wrapper a plugin author actually
/// calls, not the internal primitive.
///
/// Fail oracle: pass `name` straight through without stripping → the
/// message contains "%apply-text-edits!" instead of "apply-text-edits!".
#[test]
fn gate_strips_leading_percent_from_registered_name() {
    let mut h = SteelCtxTestHarness::new();
    let err = require_cmd(&h.ctx_init(), "%apply-text-edits!").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("apply-text-edits!"), "got: {msg}");
    assert!(!msg.contains("%apply-text-edits!"), "got: {msg}");
}

/// `generic_err` preserves the source message verbatim and constructs a
/// `Generic`-kind error (surfaced only via `Display`, not asserted
/// elsewhere — no test in this crate checks `ErrorKind`).
#[test]
fn generic_err_preserves_message() {
    let err = generic_err("buffer-path: no such buffer");
    assert!(err.to_string().contains("no such buffer"));
}
