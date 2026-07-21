use super::*;
use crate::test_support::SteelCtxTestHarness;
use steel::rvals::IntoSteelVal as _;

/// `set-option!` is blocked in plain command mode (init/plugin-load only)
/// — gated at registration time (`config` kind in `builtins!`'s table),
/// not in the body, so this tests the gate primitive directly.
///
/// Fail oracle: change `set-option!`'s table entry from `config` to
/// `open` → settings could be mutated from any command body, bypassing
/// the init-only contract.
#[test]
fn set_option_blocked_in_command_mode() {
    let mut h = SteelCtxTestHarness::new();
    let result = super::super::errors::require_config(&h.ctx(), "set-option!"); // EvalMode::Command
    assert!(result.is_err(), "set-option! must error in command mode");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("init"),
        "error must mention 'init'; got: {msg}"
    );
}

/// `set-option!` rejects value types that are not string, bool, or integer.
///
/// Fail oracle: remove the type check → a list or void would be silently
/// stringified via `{:?}` and applied as a setting value.
#[test]
fn set_option_invalid_value_type_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    // Pass a list — not a valid value type.
    let list: SteelVal = Vec::<SteelVal>::new().into_steelval().unwrap();
    let result = set_option(&mut ctx, "tab-width".into(), list);
    assert!(
        result.is_err(),
        "set-option! must reject non-string/bool/int value"
    );
}

/// In init mode with valid args, `set-option!` reaches the host (NullHost → Err,
/// proving the guard was passed and the host was called).
///
/// Fail oracle: make the guard unconditionally reject → the host is never called
/// → the error message would contain "init" instead of "NullHost".
#[test]
fn set_option_init_mode_calls_host() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let result = set_option(&mut ctx, "tab-width".into(), SteelVal::IntV(4));
    // NullHost.set_global_option returns Err — the error must NOT be the guard error.
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        !msg.contains("only valid during"),
        "must reach the host, not the guard; got: {msg}"
    );
}

/// `set-option!` accepts all three valid value types without type-error.
#[test]
fn set_option_accepts_string_bool_int_values() {
    // We only need to reach value-string conversion without type error.
    // NullHost will reject the host call, but the type conversion is the
    // interesting path here.
    let mut h = SteelCtxTestHarness::new();

    // All three types must pass the type-check (host error is fine).
    for val in [
        SteelVal::StringV("4".into()),
        SteelVal::BoolV(true),
        SteelVal::IntV(4),
    ] {
        let mut ctx = h.ctx_init();
        let result = set_option(&mut ctx, "tab-width".into(), val);
        // NullHost returns Err, but it must NOT be a TypeMismatch error.
        if let Err(e) = result {
            assert!(
                !e.to_string().contains("must be a string, bool, or integer"),
                "valid value type must not produce a type-mismatch error"
            );
        }
    }
}

/// `get-option` is blocked during init eval (the opposite gate from
/// `set-option!`: it's a command-mode read, not an init-time write) —
/// gated at registration time (`cmd` kind), tested via the gate
/// primitive directly.
///
/// Fail oracle: change `get-option`'s table entry from `cmd` to `open` →
/// readable during init, where there is no meaningful focused buffer to
/// resolve overrides against.
#[test]
fn get_option_blocked_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    let result = super::super::errors::require_cmd(&h.ctx_init(), "get-option");
    assert!(result.is_err(), "get-option must error during init eval");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("init"),
        "error must mention 'init'; got: {msg}"
    );
}

/// In command mode, `get-option` reaches the host (`NullHost` → Err,
/// proving the guard was passed and the host was called).
///
/// Fail oracle: make the guard unconditionally reject → the error would
/// contain "init" instead of "NullHost".
#[test]
fn get_option_command_mode_calls_host() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = get_option(&mut ctx, "tab-width".into());
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        !msg.contains("not available during init"),
        "must reach the host, not the guard; got: {msg}"
    );
}
