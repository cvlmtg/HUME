//! `(set-option! key value)` / `(get-option key)` builtins.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::host::OptionValue;

use super::{require_cmd_ctx, require_config_ctx};

type SteelResult = Result<SteelVal, SteelErr>;

/// `(set-option! key value)`
///
/// Sets the global setting `key` to `value`. The value may be a Steel string,
/// boolean, or integer — it is converted to a string and forwarded to the
/// editor's settings layer.
///
/// Only `Global` scope is supported from scripts. Use `:set buffer …` from the
/// command line to override a setting for the active buffer.
///
/// Valid during `init.scm` or any plugin activation (init or runtime); raises
/// a Steel error if called from a plain command body.
pub(crate) fn set_option(ctx: &mut SteelCtx, key: String, value: SteelVal) -> SteelResult {
    require_config_ctx!(ctx, "set-option!");

    // Accept string, bool, or integer for `value` and convert to the string
    // representation that the settings layer expects.
    let value_str = match &value {
        SteelVal::StringV(s) => s.to_string(),
        SteelVal::BoolV(b) => b.to_string(),
        SteelVal::IntV(n) => n.to_string(),
        _ => steel::stop!(TypeMismatch =>
            "set-option!: second arg (value) must be a string, bool, or integer, got {:?}", value),
    };

    ctx.host
        .settings()
        .set_global_option(&key, &value_str)
        .map_err(|e| steel::rerrs::SteelErr::new(steel::rerrs::ErrorKind::Generic, e))?;

    Ok(SteelVal::Void)
}

/// `(get-option key)`
///
/// The effective value of `key`: the focused buffer's override if one is
/// set, else the global default. Unlike `set-option!`, callable from any
/// command-mode context — command bodies, hook handlers, timer thunks — not
/// just init/plugin-load, since features read settings (e.g. `tab-width`,
/// `lsp.inlay-hints`) while composing a request, not just at startup.
pub(crate) fn get_option(ctx: &mut SteelCtx, key: String) -> SteelResult {
    require_cmd_ctx!(ctx, "get-option");
    let value = ctx
        .host
        .settings()
        .get_option(&key, ctx.focused_buffer_id)
        .map_err(|e| SteelErr::new(steel::rerrs::ErrorKind::Generic, e))?;
    Ok(match value {
        OptionValue::Bool(b) => SteelVal::BoolV(b),
        OptionValue::Int(n) => SteelVal::IntV(n as isize),
        OptionValue::Str(s) => SteelVal::StringV(s.into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::SteelCtxTestHarness;
    use steel::rvals::IntoSteelVal as _;

    /// `set-option!` is blocked in plain command mode (init/plugin-load only).
    ///
    /// Fail oracle: remove the require_config_ctx! guard → settings can be
    /// mutated from any command body, bypassing the init-only contract.
    #[test]
    fn set_option_blocked_in_command_mode() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx(); // EvalMode::Command
        let result = set_option(&mut ctx, "tab-width".into(), SteelVal::IntV(2));
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
    /// `set-option!`: it's a command-mode read, not an init-time write).
    ///
    /// Fail oracle: remove the `require_cmd_ctx!` guard → readable during
    /// init, where there is no meaningful focused buffer to resolve
    /// overrides against.
    #[test]
    fn get_option_blocked_in_init_mode() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        let result = get_option(&mut ctx, "tab-width".into());
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
}
