//! `(bind-key! mode key-sequence command-name)`, `(bind-key-extend! …)`,
//! `(unbind-key! mode key-sequence)`, and `(bind-wait-char! …)` builtins.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::host::BindMode;
use crate::keys::parse_key_sequence;

use super::require_config_ctx;

type SteelResult = Result<SteelVal, SteelErr>;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn mode_from_symbol(mode: &SteelVal, fn_name: &str) -> Result<BindMode, SteelErr> {
    let mode_str = match mode {
        SteelVal::SymbolV(s) => s.to_string(),
        _ => steel::stop!(TypeMismatch =>
            "{fn_name}: expected a mode symbol like 'normal, got {:?}", mode),
    };
    match mode_str.as_str() {
        "normal" => Ok(BindMode::Normal),
        "extend" => Ok(BindMode::Extend),
        "insert" => Ok(BindMode::Insert),
        _ => steel::stop!(Generic =>
            "{fn_name}: unknown mode '{}'; expected normal, extend, or insert", mode_str),
    }
}

enum BindKind {
    Normal,
    WaitChar,
}

fn bind_inner(
    ctx: &mut SteelCtx,
    fn_name: &str,
    mode: SteelVal,
    key_str: String,
    cmd_name: String,
    kind: BindKind,
    force_extend: bool,
) -> SteelResult {
    require_config_ctx!(ctx, fn_name);
    let mode = mode_from_symbol(&mode, fn_name)?;
    let keys = parse_key_sequence(&key_str)
        .map_err(|e| steel::rerrs::SteelErr::new(steel::rerrs::ErrorKind::Generic, e))?;
    match kind {
        BindKind::Normal => ctx
            .host
            .keymap()
            .bind_key(mode, &keys, &cmd_name, force_extend)
            .map_err(|e| steel::rerrs::SteelErr::new(steel::rerrs::ErrorKind::Generic, e))?,
        BindKind::WaitChar => ctx
            .host
            .keymap()
            .bind_wait_char(mode, &keys, &cmd_name)
            .map_err(|e| steel::rerrs::SteelErr::new(steel::rerrs::ErrorKind::Generic, e))?,
    }
    Ok(SteelVal::Void)
}

// ── Builtins ──────────────────────────────────────────────────────────────────

/// `(bind-key! 'mode key-sequence command-name)`
///
/// Binds a key sequence in the given mode to a named command.
///
/// - `mode` — a symbol: `'normal`, `'extend`, or `'insert`.
/// - `key-sequence` — a string parsed by [`parse_key_sequence`].
/// - `command-name` — the canonical command name (must be registered in
///   the [`CommandRegistry`] at dispatch time; not validated here).
///
/// Only valid during `init.scm` or plugin load.
pub(crate) fn bind_key(
    ctx: &mut SteelCtx,
    mode: SteelVal,
    key_str: String,
    cmd_name: String,
) -> SteelResult {
    bind_inner(
        ctx,
        "bind-key!",
        mode,
        key_str,
        cmd_name,
        BindKind::Normal,
        false,
    )
}

/// `(bind-key-extend! 'mode key-sequence command-name)`
///
/// Like `(bind-key! …)` but marks the binding as always-extending
/// (`force_extend = true`). The command will extend the selection whenever
/// this key is pressed in Normal mode, without requiring sticky Extend mode.
///
/// Only valid during `init.scm` or plugin load.
pub(crate) fn bind_key_extend(
    ctx: &mut SteelCtx,
    mode: SteelVal,
    key_str: String,
    cmd_name: String,
) -> SteelResult {
    bind_inner(
        ctx,
        "bind-key-extend!",
        mode,
        key_str,
        cmd_name,
        BindKind::Normal,
        true,
    )
}

/// `(unbind-key! 'mode key-sequence)`
///
/// Removes the binding for `key-sequence` in `mode`. Silent no-op if the
/// sequence is already unbound. Only valid during `init.scm` or plugin load.
pub(crate) fn unbind_key(ctx: &mut SteelCtx, mode: SteelVal, key_str: String) -> SteelResult {
    require_config_ctx!(ctx, "unbind-key!");
    let mode = mode_from_symbol(&mode, "unbind-key!")?;
    let keys = parse_key_sequence(&key_str)
        .map_err(|e| steel::rerrs::SteelErr::new(steel::rerrs::ErrorKind::Generic, e))?;
    ctx.host
        .keymap()
        .unbind_key(mode, &keys)
        .map_err(|e| steel::rerrs::SteelErr::new(steel::rerrs::ErrorKind::Generic, e))?;
    Ok(SteelVal::Void)
}

/// `(bind-wait-char! 'mode key-sequence command-name)`
///
/// Binds a key sequence to a WaitChar node so that after the user completes
/// the sequence, the next character is stored in `pending_char` and
/// `command-name` is dispatched.
///
/// Only valid during `init.scm` or plugin load.
pub(crate) fn bind_wait_char(
    ctx: &mut SteelCtx,
    mode: SteelVal,
    key_str: String,
    cmd_name: String,
) -> SteelResult {
    bind_inner(
        ctx,
        "bind-wait-char!",
        mode,
        key_str,
        cmd_name,
        BindKind::WaitChar,
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::SteelCtxTestHarness;

    // ── Init-only guard ───────────────────────────────────────────────────────

    /// `bind-key!` is blocked in plain command mode (is_init=false, plugin_stack empty).
    ///
    /// Fail oracle: remove the guard → a plugin command body could rebind keys at runtime.
    #[test]
    fn bind_key_blocked_in_command_mode() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let result = bind_key(
            &mut ctx,
            SteelVal::SymbolV("normal".into()),
            "z".into(),
            "move-right".into(),
        );
        assert!(result.is_err(), "bind-key! must error in command mode");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("init"),
            "error must mention 'init'; got: {msg}"
        );
    }

    /// `bind-key-extend!` is blocked in plain command mode.
    #[test]
    fn bind_key_extend_blocked_in_command_mode() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let result = bind_key_extend(
            &mut ctx,
            SteelVal::SymbolV("normal".into()),
            "z".into(),
            "move-right".into(),
        );
        assert!(
            result.is_err(),
            "bind-key-extend! must error in command mode"
        );
    }

    /// `unbind-key!` is blocked in plain command mode.
    #[test]
    fn unbind_key_blocked_in_command_mode() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let result = unbind_key(&mut ctx, SteelVal::SymbolV("normal".into()), "z".into());
        assert!(result.is_err(), "unbind-key! must error in command mode");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("init"),
            "error must mention 'init'; got: {msg}"
        );
    }

    /// `bind-wait-char!` is blocked in plain command mode.
    #[test]
    fn bind_wait_char_blocked_in_command_mode() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        let result = bind_wait_char(
            &mut ctx,
            SteelVal::SymbolV("normal".into()),
            "f".into(),
            "wait-f".into(),
        );
        assert!(
            result.is_err(),
            "bind-wait-char! must error in command mode"
        );
    }

    // ── Mode validation ───────────────────────────────────────────────────────

    /// `bind-key!` rejects an unknown mode name.
    ///
    /// Fail oracle: remove `mode_from_symbol` validation → 'visual silently picks an
    /// arbitrary arm in the match and inserts into the wrong trie.
    #[test]
    fn bind_key_invalid_mode_errors() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        let result = bind_key(
            &mut ctx,
            SteelVal::SymbolV("visual".into()),
            "z".into(),
            "move-right".into(),
        );
        assert!(result.is_err(), "bind-key! must reject unknown mode");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("mode"),
            "error must mention 'mode'; got: {msg}"
        );
    }

    /// `unbind-key!` rejects an unknown mode name.
    #[test]
    fn unbind_key_invalid_mode_errors() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        let result = unbind_key(&mut ctx, SteelVal::SymbolV("visual".into()), "z".into());
        assert!(result.is_err(), "unbind-key! must reject unknown mode");
    }

    /// `bind-key!` rejects a string mode (the pre-migration convention);
    /// mode must now be a symbol.
    ///
    /// Fail oracle: if `mode_from_symbol` still coerced strings, this would pass
    /// silently instead of raising a type mismatch.
    #[test]
    fn bind_key_string_mode_errors() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        let result = bind_key(
            &mut ctx,
            SteelVal::StringV("normal".into()),
            "z".into(),
            "move-right".into(),
        );
        assert!(result.is_err(), "bind-key! must reject a string mode");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("symbol"),
            "error must mention 'symbol'; got: {msg}"
        );
    }

    // ── Key-sequence parsing ──────────────────────────────────────────────────

    /// `bind-key!` rejects an invalid key-sequence string.
    ///
    /// Fail oracle: short-circuit key parsing to always return Ok([]) →
    /// the binding is silently inserted under an empty key, which is unreachable.
    #[test]
    fn bind_key_invalid_key_sequence_errors() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        // "<NOTAKEY>" is not a valid key name.
        let result = bind_key(
            &mut ctx,
            SteelVal::SymbolV("normal".into()),
            "<NOTAKEY>".into(),
            "move-right".into(),
        );
        assert!(
            result.is_err(),
            "bind-key! must reject invalid key sequences"
        );
    }

    /// `unbind-key!` also validates the key sequence.
    #[test]
    fn unbind_key_invalid_key_sequence_errors() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        let result = unbind_key(
            &mut ctx,
            SteelVal::SymbolV("normal".into()),
            "<NOTAKEY>".into(),
        );
        assert!(
            result.is_err(),
            "unbind-key! must reject invalid key sequences"
        );
    }

    // ── Guard passes, host called ─────────────────────────────────────────────

    /// In init mode with valid args, `bind-key!` passes the guard and reaches the
    /// host.  NullHost returns Err("NullHost: bind_key not available"), which proves
    /// the error is NOT the guard error.
    #[test]
    fn bind_key_init_mode_calls_host() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        let result = bind_key(
            &mut ctx,
            SteelVal::SymbolV("normal".into()),
            "z".into(),
            "move-right".into(),
        );
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            !msg.contains("only valid during"),
            "must reach the host, not the guard; got: {msg}"
        );
    }

    // ── Plugin-load context (plugin_stack non-empty) ──────────────────────────

    /// When `plugin_stack` is non-empty (inside a plugin body), all four bind
    /// builtins are permitted even with `is_init = false`.
    #[test]
    fn bind_key_permitted_during_plugin_load() {
        use crate::attribution::PluginId;
        let mut h = SteelCtxTestHarness::new();
        h.plugin_stack
            .push(PluginId::parse("core:myplugin").unwrap());
        {
            let mut ctx = h.ctx(); // is_init=false, plugin_stack non-empty → allowed
            let result = bind_key(
                &mut ctx,
                SteelVal::SymbolV("normal".into()),
                "z".into(),
                "cmd".into(),
            );
            // Guard must pass; NullHost error is expected.
            assert!(result.is_err());
            assert!(
                !result
                    .unwrap_err()
                    .to_string()
                    .contains("only valid during"),
                "bind-key! must not hit the init guard during plugin load"
            );
        }
    }
}
