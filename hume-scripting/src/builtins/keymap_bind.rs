//! `(bind-key! mode key-sequence command-name [#:keymap name])`,
//! `(bind-key-extend! …)`, `(unbind-key! mode key-sequence [#:keymap name])`,
//! `(attach-keymap! pane name)` and `(detach-keymap! pane name)` builtins.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::host::{BindMode, KeymapTarget};
use crate::keys::parse_key_sequence;
use crate::{Effect, PaneHandle, SteelCtx};

use super::SteelResult;
use super::args::{optional_symbol_arg, symbol_arg, symbol_enum_arg};
use super::errors::{self, generic_err};

// ── Helpers ───────────────────────────────────────────────────────────────────

/// The mode symbols the bind builtins take.
const MODES: [(&str, BindMode); 4] = [
    ("normal", BindMode::Normal),
    ("extend", BindMode::Extend),
    ("insert", BindMode::Insert),
    ("command", BindMode::Command),
];

fn mode_from_symbol(mode: &SteelVal) -> Result<BindMode, SteelErr> {
    let mode_str = symbol_arg(mode, "mode", "a mode symbol like 'normal")?;
    symbol_enum_arg(mode_str, "mode", &MODES)
}

fn mode_symbol(mode: BindMode) -> &'static str {
    MODES
        .iter()
        .find(|(_, m)| *m == mode)
        .map(|(name, _)| *name)
        .expect("every BindMode has a symbol in MODES")
}

/// The keymap a `#:keymap` argument names. The global keymap only changes
/// during `init.scm` or plugin load; a named one may change from anywhere.
fn keymap_target(ctx: &SteelCtx, keymap: SteelVal) -> Result<KeymapTarget, SteelErr> {
    match optional_symbol_arg(keymap, "#:keymap")? {
        Some(name) => Ok(KeymapTarget::Named(name)),
        None => {
            errors::require_config(ctx)?;
            Ok(KeymapTarget::Global)
        }
    }
}

fn bind_inner(
    ctx: &mut SteelCtx,
    mode: SteelVal,
    key_str: String,
    cmd_name: String,
    force_extend: bool,
    keymap: SteelVal,
) -> SteelResult {
    let keymap = keymap_target(ctx, keymap)?;
    let mode = mode_from_symbol(&mode)?;
    let keys = parse_key_sequence(&key_str).map_err(generic_err)?;
    // The editor walks these two keymaps one key at a time, so a sequence
    // bound there could never fire.
    if keys.len() > 1 && matches!(mode, BindMode::Insert | BindMode::Command) {
        steel::stop!(Generic =>
            "a '{} binding takes a single key, got {key_str:?}", mode_symbol(mode));
    }
    if force_extend && mode != BindMode::Normal {
        steel::stop!(Generic =>
            "only a 'normal binding can extend, got '{}", mode_symbol(mode));
    }
    // Queued, not applied: a failed plugin activation's binds are dropped by
    // `pop_effect_marks(false)` before the editor ever sees them. Validation
    // above still fails synchronously: a bad mode or key sequence is the
    // script's bug, not a side effect to defer.
    ctx.push_effect(Effect::BindKey {
        mode,
        keys,
        cmd: cmd_name,
        force_extend,
        keymap,
    });
    Ok(SteelVal::Void)
}

// ── Builtins ──────────────────────────────────────────────────────────────────

/// `(bind-key! 'mode key-sequence command-name [#:keymap name])`
///
/// Binds a key sequence in the given mode to a named command, in the global
/// keymap or, with `#:keymap`, in the named keymap, creating it if needed.
///
/// - `mode`: a symbol: `'normal`, `'extend`, `'insert`, or `'command`.
///   `'insert` and `'command` take a single key.
/// - `key-sequence`: a string parsed by [`parse_key_sequence`].
/// - `command-name`: the canonical command name (must be registered in
///   the editor's `CommandRegistry` at dispatch time; not validated here).
/// - `name`: a symbol, or `#f` for the global keymap.
///
/// A global bind is only valid during `init.scm` or plugin load.
pub(crate) fn bind_key(
    ctx: &mut SteelCtx,
    mode: SteelVal,
    key_str: String,
    cmd_name: String,
    keymap: SteelVal,
) -> SteelResult {
    bind_inner(ctx, mode, key_str, cmd_name, false, keymap)
}

/// `(bind-key-extend! 'mode key-sequence command-name [#:keymap name])`
///
/// Like `(bind-key! …)` but marks the binding as always-extending
/// (`force_extend = true`). The command will extend the selection whenever
/// this key is pressed in Normal mode, without requiring sticky Extend mode.
/// Raises for any other mode, where nothing reads the flag.
///
/// A global bind is only valid during `init.scm` or plugin load.
pub(crate) fn bind_key_extend(
    ctx: &mut SteelCtx,
    mode: SteelVal,
    key_str: String,
    cmd_name: String,
    keymap: SteelVal,
) -> SteelResult {
    bind_inner(ctx, mode, key_str, cmd_name, true, keymap)
}

/// `(unbind-key! 'mode key-sequence [#:keymap name])`
///
/// Removes the binding for `key-sequence` in `mode`, from the global keymap
/// or the named one. A no-op if the sequence is already unbound. A global
/// unbind is only valid during `init.scm` or plugin load.
pub(crate) fn unbind_key(
    ctx: &mut SteelCtx,
    mode: SteelVal,
    key_str: String,
    keymap: SteelVal,
) -> SteelResult {
    let keymap = keymap_target(ctx, keymap)?;
    let mode = mode_from_symbol(&mode)?;
    let keys = parse_key_sequence(&key_str).map_err(generic_err)?;
    ctx.push_effect(Effect::UnbindKey { mode, keys, keymap });
    Ok(SteelVal::Void)
}

fn keymap_name(name: SteelVal) -> Result<String, SteelErr> {
    symbol_arg(&name, "name", "a keymap name symbol like 'my-keys").map(str::to_owned)
}

/// `(attach-keymap! pane name)`
///
/// Makes the keymap `name` apply in `pane`'s buffer, ahead of the keymaps
/// attached before it. A keymap already attached keeps its place. The editor
/// warns when no keymap has that name yet.
pub(crate) fn attach_keymap(ctx: &mut SteelCtx, pane: PaneHandle, name: SteelVal) -> SteelResult {
    let name = keymap_name(name)?;
    ctx.push_effect(Effect::AttachKeymap {
        buffer: pane.buffer(),
        name,
    });
    Ok(SteelVal::Void)
}

/// `(detach-keymap! pane name)`
///
/// Stops the keymap `name` applying in `pane`'s buffer. The editor warns when
/// no keymap has that name.
pub(crate) fn detach_keymap(ctx: &mut SteelCtx, pane: PaneHandle, name: SteelVal) -> SteelResult {
    let name = keymap_name(name)?;
    ctx.push_effect(Effect::DetachKeymap {
        buffer: pane.buffer(),
        name,
    });
    Ok(SteelVal::Void)
}

#[cfg(test)]
mod tests;
