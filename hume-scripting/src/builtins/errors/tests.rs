use super::*;
use crate::attribution::{EntryId, PluginId};
use crate::test_support::SteelCtxTestHarness;

/// `require_cmd` rejects `Init` and `PluginLoad`, allows the other two.
///
/// The expected pass/fail per state comes from `EvalMode`'s doc table.
#[test]
fn require_cmd_gates_by_mode() {
    let mut h = SteelCtxTestHarness::new();
    assert!(require_cmd(&h.ctx_init()).is_err(), "Init must reject");
    assert!(require_cmd(&h.ctx()).is_ok(), "Command must pass");

    h.attribution
        .push_body(EntryId::main(PluginId::parse("core:test-plugin").unwrap()));
    assert!(
        require_cmd(&h.ctx_init()).is_err(),
        "PluginLoad must reject"
    );
    assert!(
        require_cmd(&h.ctx_activation()).is_ok(),
        "PluginActivation must pass"
    );
}

/// `require_config` rejects only `Command`, allows the other three.
#[test]
fn require_config_gates_by_mode() {
    let mut h = SteelCtxTestHarness::new();
    assert!(require_config(&h.ctx_init()).is_ok(), "Init must pass");
    assert!(require_config(&h.ctx()).is_err(), "Command must reject");

    h.attribution
        .push_body(EntryId::main(PluginId::parse("core:test-plugin").unwrap()));
    assert!(
        require_config(&h.ctx_init()).is_ok(),
        "PluginLoad must pass"
    );
    assert!(
        require_config(&h.ctx_activation()).is_ok(),
        "PluginActivation must pass"
    );
}

/// `named` puts the builtin's name in front of an error and keeps its kind.
#[test]
fn named_starts_the_error_with_the_builtin() {
    let err = named::<()>("buffer-path", || Err(generic_err("no such buffer"))).unwrap_err();
    assert_eq!(
        err.to_string(),
        "Error: Generic: buffer-path: no such buffer"
    );
}

/// A message that names its own builtin is a bug the funnel refuses.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "buffer-path named itself in its error")]
fn named_refuses_a_message_that_names_its_builtin() {
    assert!(
        named::<()>("buffer-path", || Err(generic_err(
            "buffer-path: no such buffer"
        )))
        .is_err()
    );
}

/// `generic_err` preserves the source message verbatim and constructs a
/// `Generic`-kind error (surfaced only via `Display`, not asserted
/// elsewhere; no test in this crate checks `ErrorKind`).
#[test]
fn generic_err_preserves_message() {
    let err = generic_err("buffer-path: no such buffer");
    assert!(err.to_string().contains("no such buffer"));
}
