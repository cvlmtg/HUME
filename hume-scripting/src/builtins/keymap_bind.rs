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

/// A WaitChar bind has no `force_extend` notion (see `Effect::BindWaitChar`'s
/// doc) — carried on the variant, not as a sibling parameter, so the illegal
/// combination can't be constructed.
enum BindKind {
    Normal { force_extend: bool },
    WaitChar,
}

fn bind_inner(
    ctx: &mut SteelCtx,
    fn_name: &str,
    mode: SteelVal,
    key_str: String,
    cmd_name: String,
    kind: BindKind,
) -> SteelResult {
    let mode = mode_from_symbol(&mode, fn_name)?;
    let keys = parse_key_sequence(&key_str).map_err(generic_err)?;
    // Queued, not applied: a failed plugin activation's binds are dropped by
    // `pop_effect_marks(false)` before the editor ever sees them. Validation
    // above still fails synchronously — a bad mode or key sequence is the
    // script's bug, not a side effect to defer.
    ctx.push_effect(match kind {
        BindKind::Normal { force_extend } => Effect::BindKey {
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
///   the editor's `CommandRegistry` at dispatch time; not validated here).
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
        BindKind::Normal { force_extend: false },
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
        BindKind::Normal { force_extend: true },
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
    bind_inner(ctx, "bind-wait-char!", mode, key_str, cmd_name, BindKind::WaitChar)
}

#[cfg(test)]
mod tests;
