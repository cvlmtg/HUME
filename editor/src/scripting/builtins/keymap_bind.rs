//! `(bind-key! mode key-sequence command-name)` and `(bind-wait-char! …)` builtins.
//!
//! The key-string parser lives in [`crate::scripting::keys`]; this module
//! forwards the `key-sequence` argument to it and handles ledger recording
//! for plugin-attributed mutations.

use std::borrow::Cow;

use crossterm::event::KeyEvent;
use steel::rvals::SteelVal;
use steel::rerrs::SteelErr;

use crate::editor::keymap::BindMode;
use crate::scripting::keys::parse_key_sequence;
use crate::scripting::ledger::Owner;

type SteelResult = Result<SteelVal, SteelErr>;

// ── Builtins ──────────────────────────────────────────────────────────────────

/// Parsed arguments shared by `bind_key` and `bind_wait_char`.
struct ParsedBindArgs {
    mode:       BindMode,
    /// `"<mode_lower> <key_str>"` — ledger key encoding both so that
    /// `"normal f"` and `"insert f"` are tracked independently.
    ledger_key: String,
    keys:       Vec<KeyEvent>,
    cmd_name:   String,
}

/// Validate and extract the three arguments common to `bind-key!` and
/// `bind-wait-char!`: `(mode key-sequence command-name)`.
fn parse_bind_args(args: &[SteelVal], fn_name: &str) -> Result<ParsedBindArgs, SteelErr> {
    if args.len() != 3 {
        steel::stop!(ArityMismatch =>
            "{fn_name} expects 3 args (mode key-sequence command-name), got {}", args.len());
    }
    let mode_str = match &args[0] {
        SteelVal::StringV(s) => s.to_string(),
        _ => steel::stop!(TypeMismatch =>
            "{fn_name}: first arg (mode) must be a string, got {:?}", args[0]),
    };
    let key_str = match &args[1] {
        SteelVal::StringV(s) => s.to_string(),
        _ => steel::stop!(TypeMismatch =>
            "{fn_name}: second arg (key-sequence) must be a string, got {:?}", args[1]),
    };
    let cmd_name = match &args[2] {
        SteelVal::StringV(s) => s.to_string(),
        _ => steel::stop!(TypeMismatch =>
            "{fn_name}: third arg (command-name) must be a string, got {:?}", args[2]),
    };
    let mode = match mode_str.to_ascii_lowercase().as_str() {
        "normal" => BindMode::Normal,
        "extend" => BindMode::Extend,
        "insert" => BindMode::Insert,
        _ => steel::stop!(Generic =>
            "{fn_name}: unknown mode '{}'; expected normal, extend, or insert", mode_str),
    };
    let keys = parse_key_sequence(&key_str)
        .map_err(|e| steel::rerrs::SteelErr::new(steel::rerrs::ErrorKind::Generic, e))?;
    let ledger_key = format!("{} {}", mode_str.to_ascii_lowercase(), key_str);
    Ok(ParsedBindArgs { mode, ledger_key, keys, cmd_name })
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
pub(crate) fn bind_key(args: &[SteelVal]) -> SteelResult {
    let ParsedBindArgs { mode, ledger_key, keys, cmd_name } = parse_bind_args(args, "bind-key!")?;
    super::with_ctx("bind-key!", |ctx| {
        let prior_value = ctx.keymap.lookup_command(mode, &keys).unwrap_or_default();
        let prior_owner = ctx.ledger_stack.owner_of(&ledger_key);
        let current_owner = ctx.plugin_stack.current_owner();
        ctx.keymap.bind_user(mode, &keys, Cow::Owned(cmd_name));
        if let Owner::Plugin(ref plugin_id) = current_owner {
            ctx.ledger_stack.record(plugin_id, ledger_key, prior_owner, prior_value);
        }
        Ok(SteelVal::Void)
    })
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
pub(crate) fn bind_wait_char(args: &[SteelVal]) -> SteelResult {
    let ParsedBindArgs { mode, ledger_key, keys, cmd_name } = parse_bind_args(args, "bind-wait-char!")?;
    super::with_ctx("bind-wait-char!", |ctx| {
        let prior_value = ctx.keymap.lookup_command(mode, &keys).unwrap_or_default();
        let prior_owner = ctx.ledger_stack.owner_of(&ledger_key);
        let current_owner = ctx.plugin_stack.current_owner();
        ctx.keymap.bind_wait_char_user(mode, &keys, Cow::Owned(cmd_name));
        if let Owner::Plugin(ref plugin_id) = current_owner {
            ctx.ledger_stack.record(plugin_id, ledger_key, prior_owner, prior_value);
        }
        Ok(SteelVal::Void)
    })
}
