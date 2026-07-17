//! `(bind-key! mode key-sequence command-name)`, `(bind-key-extend! …)`,
//! `(unbind-key! mode key-sequence)`, and `(bind-wait-char! …)` builtins.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::host::BindMode;
use crate::keys::parse_key_sequence;
use crate::{Effect, SteelCtx};

use super::errors::generic_err;

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
    let mode = mode_from_symbol(&mode, fn_name)?;
    let keys = parse_key_sequence(&key_str).map_err(generic_err)?;
    // Queued, not applied: a failed plugin activation's binds are dropped by
    // `pop_effect_marks(false)` before the editor ever sees them. Validation
    // above still fails synchronously — a bad mode or key sequence is the
    // script's bug, not a side effect to defer.
    ctx.push_effect(match kind {
        BindKind::Normal => Effect::BindKey {
            mode,
            keys,
            cmd: cmd_name,
            force_extend,
        },
        BindKind::WaitChar => Effect::BindWaitChar {
            mode,
            keys,
            cmd: cmd_name,
        },
    });
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
    let mode = mode_from_symbol(&mode, "unbind-key!")?;
    let keys = parse_key_sequence(&key_str).map_err(generic_err)?;
    ctx.push_effect(Effect::UnbindKey { mode, keys });
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

    /// Effects queued so far, in emission order.
    fn effects(h: &SteelCtxTestHarness) -> Vec<&Effect> {
        h.effects.iter().map(|e| &e.effect).collect()
    }

    // ── Init-only guard ───────────────────────────────────────────────────────
    //
    // All four bind builtins below are `config`-gated in `builtins!`'s
    // registration table — the gate lives in the registration wrapper
    // closure, not the body, so these test the gate primitive directly.

    /// `bind-key!` is blocked in plain command mode (`EvalMode::Command`).
    ///
    /// Fail oracle: change `bind-key!`'s table entry from `config` to `open`
    /// → a plugin command body could rebind keys at runtime.
    #[test]
    fn bind_key_blocked_in_command_mode() {
        let mut h = SteelCtxTestHarness::new();
        let result = super::super::errors::require_config(&h.ctx(), "bind-key!");
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
        let result = super::super::errors::require_config(&h.ctx(), "bind-key-extend!");
        assert!(
            result.is_err(),
            "bind-key-extend! must error in command mode"
        );
    }

    /// `unbind-key!` is blocked in plain command mode.
    #[test]
    fn unbind_key_blocked_in_command_mode() {
        let mut h = SteelCtxTestHarness::new();
        let result = super::super::errors::require_config(&h.ctx(), "unbind-key!");
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
        let result = super::super::errors::require_config(&h.ctx(), "bind-wait-char!");
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
        drop(ctx);
        assert!(
            effects(&h).is_empty(),
            "validation must reject before queueing anything"
        );
    }

    /// `unbind-key!` rejects an unknown mode name.
    #[test]
    fn unbind_key_invalid_mode_errors() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        let result = unbind_key(&mut ctx, SteelVal::SymbolV("visual".into()), "z".into());
        assert!(result.is_err(), "unbind-key! must reject unknown mode");
        drop(ctx);
        assert!(
            effects(&h).is_empty(),
            "validation must reject before queueing anything"
        );
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
        drop(ctx);
        assert!(
            effects(&h).is_empty(),
            "validation must reject before queueing anything"
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
        drop(ctx);
        assert!(
            effects(&h).is_empty(),
            "validation must reject before queueing anything"
        );
    }

    // ── Guard passes, effect queued ───────────────────────────────────────────

    /// In init mode with valid args, `bind-key!` passes the guard and queues an
    /// `Effect::BindKey` carrying the parsed mode/keys/command — nothing touches
    /// the keymap at builtin time; the editor applies it later via
    /// `Editor::apply_script_effects`.
    ///
    /// Fail oracle: drop the `push_effect` from `bind_inner` → `effects` comes
    /// back empty and the binding vanishes silently.
    #[test]
    fn bind_key_init_mode_queues_bind_effect() {
        let mut h = SteelCtxTestHarness::new();
        {
            let mut ctx = h.ctx_init();
            let result = bind_key(
                &mut ctx,
                SteelVal::SymbolV("normal".into()),
                "z".into(),
                "move-right".into(),
            );
            assert!(result.is_ok(), "bind-key! must succeed; got: {result:?}");
        }
        assert!(
            matches!(
                effects(&h).as_slice(),
                [Effect::BindKey { mode: BindMode::Normal, keys, cmd, force_extend: false }]
                    if keys.len() == 1 && cmd == "move-right"
            ),
            "expected one Effect::BindKey for 'z' → move-right; got: {:?}",
            effects(&h)
        );
    }

    /// `bind-key-extend!` queues the same effect with `force_extend: true` —
    /// the flag is the only thing distinguishing it from `bind-key!`.
    ///
    /// Fail oracle: hardcode `force_extend: false` in `bind_inner`'s
    /// `Effect::BindKey` → this fires while `bind_key_init_mode_queues_bind_effect`
    /// still passes.
    #[test]
    fn bind_key_extend_queues_force_extend_effect() {
        let mut h = SteelCtxTestHarness::new();
        {
            let mut ctx = h.ctx_init();
            bind_key_extend(
                &mut ctx,
                SteelVal::SymbolV("normal".into()),
                "z".into(),
                "select-line".into(),
            )
            .expect("bind-key-extend! must succeed");
        }
        assert!(
            matches!(
                effects(&h).as_slice(),
                [Effect::BindKey { force_extend: true, cmd, .. }] if cmd == "select-line"
            ),
            "expected Effect::BindKey with force_extend: true; got: {:?}",
            effects(&h)
        );
    }

    /// `bind-wait-char!` queues `Effect::BindWaitChar`, not `Effect::BindKey` —
    /// a WaitChar node consumes the next keypress as an argument, so routing it
    /// to a plain leaf would silently break `pending-char`.
    ///
    /// Fail oracle: collapse `bind_inner`'s `BindKind` match to always emit
    /// `BindKey` → the binding becomes a plain leaf and this fires.
    #[test]
    fn bind_wait_char_queues_wait_char_effect() {
        let mut h = SteelCtxTestHarness::new();
        {
            let mut ctx = h.ctx_init();
            bind_wait_char(
                &mut ctx,
                SteelVal::SymbolV("normal".into()),
                "f".into(),
                "find-char".into(),
            )
            .expect("bind-wait-char! must succeed");
        }
        assert!(
            matches!(
                effects(&h).as_slice(),
                [Effect::BindWaitChar { mode: BindMode::Normal, cmd, .. }] if cmd == "find-char"
            ),
            "expected Effect::BindWaitChar; got: {:?}",
            effects(&h)
        );
    }

    /// `unbind-key!` queues `Effect::UnbindKey` — deferred like the three
    /// binders so a same-eval bind-then-unbind on one key applies in Steel's
    /// emission order rather than the unbind racing ahead.
    ///
    /// Fail oracle: revert `unbind_key` to a direct host call → `effects` comes
    /// back empty and the unbind is dropped.
    #[test]
    fn unbind_key_queues_unbind_effect() {
        let mut h = SteelCtxTestHarness::new();
        {
            let mut ctx = h.ctx_init();
            unbind_key(&mut ctx, SteelVal::SymbolV("normal".into()), "z".into())
                .expect("unbind-key! must succeed");
        }
        assert!(
            matches!(
                effects(&h).as_slice(),
                [Effect::UnbindKey { mode: BindMode::Normal, keys }] if keys.len() == 1
            ),
            "expected Effect::UnbindKey; got: {:?}",
            effects(&h)
        );
    }

    // ── Plugin-load context (plugin_stack non-empty) ──────────────────────────

    /// When `plugin_stack` is non-empty (inside a plugin body), all four bind
    /// builtins are permitted even in `EvalMode::PluginActivation`.
    #[test]
    fn bind_key_permitted_during_plugin_load() {
        use crate::attribution::PluginId;
        let mut h = SteelCtxTestHarness::new();
        h.plugin_stack
            .push(PluginId::parse("core:myplugin").unwrap());
        {
            let mut ctx = h.ctx(); // EvalMode::PluginActivation → allowed
            let result = bind_key(
                &mut ctx,
                SteelVal::SymbolV("normal".into()),
                "z".into(),
                "cmd".into(),
            );
            assert!(
                result.is_ok(),
                "bind-key! must not hit the init guard during plugin load; got: {result:?}"
            );
        }
        assert!(
            matches!(effects(&h).as_slice(), [Effect::BindKey { .. }]),
            "expected the bind to be queued; got: {:?}",
            effects(&h)
        );
    }
}
