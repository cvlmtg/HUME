//! `(set-option! key value)` / `(get-option key)` builtins.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::host::OptionValue;

use super::args::BidArg;
use super::errors::generic_err;

type SteelResult = Result<SteelVal, SteelErr>;

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
/// boolean, or integer — it is converted to a string and forwarded to the
/// editor's settings layer.
///
/// Only `Global` scope is supported from scripts. Use `:set buffer …` from the
/// command line, or `(set-buffer-option! bid key value)` from a script, to
/// override a setting for a specific buffer.
///
/// Valid during `init.scm` or any plugin activation (init or runtime); raises
/// a Steel error if called from a plain command body.
pub(crate) fn set_option(ctx: &mut SteelCtx, key: String, value: SteelVal) -> SteelResult {
    let value_str = coerce_option_value(&value, "set-option!")?;

    ctx.host
        .settings()
        .set_global_option(&key, &value_str)
        .map_err(generic_err)?;

    Ok(SteelVal::Void)
}

/// `(set-buffer-option! bid key value)`
///
/// Sets `key`'s per-buffer override on `bid` to `value` (same string/bool/int
/// coercion as `set-option!`). The override persists on the buffer until
/// overwritten, same as `:set buffer key=value`.
///
/// `key` must not be `"language"` — that lives on the buffer's language
/// identity, not its settings; use `(set-buffer-language! bid lang)` instead.
///
/// Command/hook context only (`cmd` kind) — the idiomatic caller is an
/// `on-language-set` hook handler, which receives the target buffer id as an
/// explicit argument rather than relying on `(current-buffer)` (a hook fires
/// with the *focused* buffer as scripting context, which may differ from the
/// buffer whose language just changed).
pub(crate) fn set_buffer_option(
    ctx: &mut SteelCtx,
    bid: BidArg,
    key: String,
    value: SteelVal,
) -> SteelResult {
    let value_str = coerce_option_value(&value, "set-buffer-option!")?;
    if key == "language" {
        steel::stop!(Generic =>
            "set-buffer-option!: 'language' is not a setting — use (set-buffer-language! bid lang)");
    }
    let id = bid.0;
    if !ctx.host.buffers().buffer_exists(id) {
        steel::stop!(Generic => "set-buffer-option!: invalid buffer id {id:?}");
    }

    ctx.host
        .settings()
        .set_buffer_option(&key, &value_str, id)
        .map_err(generic_err)?;

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
    let value = ctx
        .host
        .settings()
        .get_option(&key, ctx.focused_buffer_id)
        .map_err(generic_err)?;
    Ok(match value {
        OptionValue::Bool(b) => SteelVal::BoolV(b),
        OptionValue::Int(n) => SteelVal::IntV(n as isize),
        OptionValue::Str(s) => SteelVal::StringV(s.into()),
    })
}

#[cfg(test)]
mod tests;
