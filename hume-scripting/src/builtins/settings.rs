//! `(set-option! key value)` / `(get-option key)` / `(set-buffer-option! pane
//! key value)` / `(get-buffer-option pane key)` builtins.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::host::OptionValue;
use crate::types::PaneHandle;

use super::SteelResult;
use super::errors::generic_err;

/// Coerce a Steel string/bool/int settings value to the settings layer's
/// string wire form. `ctx_name` names the calling builtin in the error.
fn coerce_option_value(value: &SteelVal, ctx_name: &str) -> Result<String, SteelErr> {
    match value {
        SteelVal::StringV(s) => Ok(s.to_string()),
        SteelVal::BoolV(b) => Ok(b.to_string()),
        SteelVal::IntV(n) => Ok(n.to_string()),
        _ => steel::stop!(TypeMismatch =>
            "{ctx_name}: value must be a string, bool, or integer, got {:?}", value),
    }
}

/// `(set-option! key value)`
///
/// Sets the global setting `key` to `value`. The value may be a Steel string,
/// boolean, or integer. It is converted to a string and forwarded to the
/// editor's settings layer, which is the single validating chokepoint
/// (`editor::settings::ops::apply_global`) regardless of caller, so this is
/// callable from any context: `init.scm`, plugin load, plugin activation, or
/// a plain command/hook body. Use `:set buffer …` from the command line, or
/// `(set-buffer-option! pane key value)` from a script, to override a setting
/// for a specific buffer instead.
pub(crate) fn set_option(ctx: &mut SteelCtx, key: String, value: SteelVal) -> SteelResult {
    let value_str = coerce_option_value(&value, "set-option!")?;

    ctx.host
        .settings()
        .set_global_option(&key, &value_str)
        .map_err(generic_err)?;

    Ok(SteelVal::Void)
}

/// `(set-buffer-option! pane key value)`
///
/// Sets `key`'s per-buffer override on `pane`'s buffer to `value` (same string/bool/int
/// coercion as `set-option!`). The override persists on the buffer until
/// overwritten, same as `:set buffer key=value`.
///
/// `key` must not be `"language"`: that lives on the buffer's language
/// identity, not its settings; use `(set-buffer-language! pane lang)` instead.
///
/// Command/hook context only (`cmd` kind). The idiomatic caller is an
/// `on-language-set` hook handler, which receives the target buffer id as an
/// explicit argument rather than relying on `(focused-pane)` (a live read
/// that may differ from the buffer whose language just changed).
pub(crate) fn set_buffer_option(
    ctx: &mut SteelCtx,
    pane: PaneHandle,
    key: String,
    value: SteelVal,
) -> SteelResult {
    let bid = pane.buffer();
    let value_str = coerce_option_value(&value, "set-buffer-option!")?;
    if key == "language" {
        steel::stop!(Generic =>
            "set-buffer-option!: .language. is not a setting: use (set-buffer-language! pane lang)");
    }

    ctx.host
        .settings()
        .set_buffer_option(&key, &value_str, bid)
        .map_err(generic_err)?;

    Ok(SteelVal::Void)
}

fn option_value_to_steel(value: OptionValue) -> SteelVal {
    match value {
        OptionValue::Bool(b) => SteelVal::BoolV(b),
        OptionValue::Int(n) => SteelVal::IntV(n as isize),
        OptionValue::Str(s) => SteelVal::StringV(s.into()),
    }
}

/// `(get-option key)`: `key`'s global value, ignoring any buffer override
/// even if one exists (mirrors `set-option!`, `open` kind: callable from
/// any context, including `init.scm`). Use `(get-buffer-option pane key)`
/// for a specific buffer's effective value instead.
pub(crate) fn get_option(ctx: &mut SteelCtx, key: String) -> SteelResult {
    ctx.host
        .settings()
        .get_global_option(&key)
        .map(option_value_to_steel)
        .map_err(generic_err)
}

/// `(get-buffer-option pane key)`: the effective value of `key` for `pane`'s buffer:
/// its buffer override if one is set, else the global default.
///
/// Command/hook context only (`cmd` kind). The idiomatic caller is an
/// `on-language-set` hook handler, which receives the target buffer id as an
/// explicit argument rather than relying on `(focused-pane)` (a live read
/// that may differ from the buffer whose language just changed), same
/// reasoning as `set-buffer-option!`.
pub(crate) fn get_buffer_option(ctx: &mut SteelCtx, pane: PaneHandle, key: String) -> SteelResult {
    ctx.host
        .settings()
        .get_buffer_option(&key, pane.buffer())
        .map(option_value_to_steel)
        .map_err(generic_err)
}

#[cfg(test)]
mod tests;
