//! `(bind-key! mode key-sequence command-name)` and `(bind-wait-char! …)` builtins.
//!
//! The key-string parser lives in [`crate::scripting::keys`]; this module
//! forwards the `key-sequence` argument to it and handles ledger recording
//! for plugin-attributed mutations.

use std::borrow::Cow;

use steel::rvals::SteelVal;
use steel::rerrs::SteelErr;

use crate::editor::keymap::BindMode;
use crate::scripting::keys::parse_key_sequence;
use crate::scripting::{ledger::Owner, SteelCtx};

type SteelResult = Result<SteelVal, SteelErr>;

// ── Builtins ──────────────────────────────────────────────────────────────────

fn mode_from_str(mode_str: &str, fn_name: &str) -> Result<BindMode, SteelErr> {
    match mode_str.to_ascii_lowercase().as_str() {
        "normal" => Ok(BindMode::Normal),
        "extend" => Ok(BindMode::Extend),
        "insert" => Ok(BindMode::Insert),
        _ => steel::stop!(Generic =>
            "{fn_name}: unknown mode '{}'; expected normal, extend, or insert", mode_str),
    }
}

/// `(bind-key! mode key-sequence command-name)`
///
/// Binds a key sequence in the given mode to a named command.
///
/// - `mode` — `"normal"`, `"extend"`, or `"insert"` (case-insensitive).
/// - `key-sequence` — a string parsed by [`parse_key_sequence`].
/// - `command-name` — the canonical command name (must be registered in
///   the [`CommandRegistry`] at dispatch time; not validated here).
///
/// Records a ledger entry when called from a plugin body.
/// Only valid during `init.scm` or plugin load.
pub(crate) fn bind_key(ctx: &mut SteelCtx, mode_str: String, key_str: String, cmd_name: String) -> SteelResult {
    if !ctx.is_init {
        steel::stop!(Generic =>
            "bind-key!: only valid during init.scm or plugin load, not from a Steel command body");
    }
    let mode = mode_from_str(&mode_str, "bind-key!")?;
    let keys = parse_key_sequence(&key_str)
        .map_err(|e| steel::rerrs::SteelErr::new(steel::rerrs::ErrorKind::Generic, e))?;
    let ledger_key = format!("{}{key_str}", mode.ledger_prefix());
    let prior_value = ctx.keymap.lookup_command(mode, &keys).unwrap_or_default();
    let prior_owner = ctx.ledger_stack.owner_of(&ledger_key);
    let current_owner = ctx.plugin_stack.current_owner();
    ctx.keymap.bind_user(mode, &keys, Cow::Owned(cmd_name));
    if let Owner::Plugin(ref plugin_id) = current_owner {
        ctx.ledger_stack.record(plugin_id, ledger_key, prior_owner, prior_value);
    }
    Ok(SteelVal::Void)
}

/// `(bind-wait-char! mode key-sequence command-name)`
///
/// Binds a key sequence to a WaitChar node so that after the user completes
/// the sequence, the next character is stored in `pending_char` and
/// `command-name` is dispatched.
///
/// Example: `(bind-wait-char! "normal" "m d" "helix-delete-surround")` makes
/// `m d <char>` dispatch `helix-delete-surround` with `(pending-char)` = char.
///
/// Records a ledger entry when called from a plugin body.
/// Only valid during `init.scm` or plugin load.
pub(crate) fn bind_wait_char(ctx: &mut SteelCtx, mode_str: String, key_str: String, cmd_name: String) -> SteelResult {
    if !ctx.is_init {
        steel::stop!(Generic =>
            "bind-wait-char!: only valid during init.scm or plugin load, not from a Steel command body");
    }
    let mode = mode_from_str(&mode_str, "bind-wait-char!")?;
    let keys = parse_key_sequence(&key_str)
        .map_err(|e| steel::rerrs::SteelErr::new(steel::rerrs::ErrorKind::Generic, e))?;
    let ledger_key = format!("{}{key_str}", mode.ledger_prefix());
    let prior_value = ctx.keymap.lookup_command(mode, &keys).unwrap_or_default();
    let prior_owner = ctx.ledger_stack.owner_of(&ledger_key);
    let current_owner = ctx.plugin_stack.current_owner();
    ctx.keymap.bind_wait_char_user(mode, &keys, Cow::Owned(cmd_name));
    if let Owner::Plugin(ref plugin_id) = current_owner {
        ctx.ledger_stack.record(plugin_id, ledger_key, prior_owner, prior_value);
    }
    Ok(SteelVal::Void)
}
