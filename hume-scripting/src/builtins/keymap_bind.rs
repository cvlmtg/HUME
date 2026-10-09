//! `(bind-key! mode key-sequence command-name)`, `(bind-key-extend! …)`, and
//! `(unbind-key! mode key-sequence)` builtins.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::host::BindMode;
use crate::keys::parse_key_sequence;
use crate::{Effect, SteelCtx};

use super::SteelResult;
use super::args::symbol_enum_arg;
use super::errors::generic_err;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn mode_from_symbol(mode: &SteelVal, fn_name: &str) -> Result<BindMode, SteelErr> {
    let mode_str = match mode {
        SteelVal::SymbolV(s) => s.to_string(),
        _ => steel::stop!(TypeMismatch =>
            "{fn_name}: expected a mode symbol like 'normal, got {:?}", mode),
    };
    symbol_enum_arg(
        &mode_str,
        &format!("{fn_name} mode"),
        &[
            ("normal", BindMode::Normal),
            ("extend", BindMode::Extend),
            ("insert", BindMode::Insert),
        ],
    )
}

fn bind_inner(
    ctx: &mut SteelCtx,
    fn_name: &str,
    mode: SteelVal,
    key_str: String,
    cmd_name: String,
    force_extend: bool,
) -> SteelResult {
    let mode = mode_from_symbol(&mode, fn_name)?;
    let keys = parse_key_sequence(&key_str).map_err(generic_err)?;
    // Queued, not applied: a failed plugin activation's binds are dropped by
    // `pop_effect_marks(false)` before the editor ever sees them. Validation
    // above still fails synchronously: a bad mode or key sequence is the
    // script's bug, not a side effect to defer.
    ctx.push_effect(Effect::BindKey {
        mode,
        keys,
        cmd: cmd_name,
        force_extend,
    });
    Ok(SteelVal::Void)
}

// ── Builtins ──────────────────────────────────────────────────────────────────

/// `(bind-key! 'mode key-sequence command-name)`
///
/// Binds a key sequence in the given mode to a named command.
///
/// - `mode`: a symbol: `'normal`, `'extend`, or `'insert`.
/// - `key-sequence`: a string parsed by [`parse_key_sequence`].
/// - `command-name`: the canonical command name (must be registered in
///   the editor's `CommandRegistry` at dispatch time; not validated here).
///
/// Only valid during `init.scm` or plugin load.
pub(crate) fn bind_key(
    ctx: &mut SteelCtx,
    mode: SteelVal,
    key_str: String,
    cmd_name: String,
) -> SteelResult {
    bind_inner(ctx, "bind-key!", mode, key_str, cmd_name, false)
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
    bind_inner(ctx, "bind-key-extend!", mode, key_str, cmd_name, true)
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

#[cfg(test)]
mod tests;
