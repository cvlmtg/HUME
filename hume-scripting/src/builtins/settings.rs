//! `(set-option! key value)` / `(get-option [bid] key)` builtins.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::host::OptionValue;

use super::SteelResult;
use super::args::{BidArg, optional_bid_arg};
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
/// boolean, or integer — it is converted to a string and forwarded to the
/// editor's settings layer, which is the single validating chokepoint
/// (`editor::settings_ops::apply_global`) regardless of caller — so this is
/// callable from any context: `init.scm`, plugin load, plugin activation, or
/// a plain command/hook body. Use `:set buffer …` from the command line, or
/// `(set-buffer-option! bid key value)` from a script, to override a setting
/// for a specific buffer instead.
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

    ctx.host
        .settings()
        .set_buffer_option(&key, &value_str, bid.0)
        .map_err(generic_err)?;

    Ok(SteelVal::Void)
}

/// `%get-option` — the raw primitive `(get-option [bid] key)` (BOOTSTRAP)
/// wraps, dispatching on arity to fill in `bid` as `#f` for the 1-arg form.
///
/// The effective value of `key`: `bid`'s buffer override if one is set, else
/// the global default. `bid` is `#f` when the caller omitted the leading bid,
/// meaning "the focused buffer" — mirrors `set-buffer-option!`'s explicit-bid
/// contract, so a hook handler that received a *different* buffer as an
/// argument (e.g. `on-language-set`, whose bid may differ from the focused
/// buffer) can read that buffer's settings instead of silently reading the
/// wrong one.
///
/// Callable from any context — command bodies, hook handlers, timer thunks,
/// `init.scm`, plugin load — since features read settings (e.g. `tab-width`,
/// `lsp.inlay-hints`) while composing a request, not just at startup, and a
/// stale or default buffer id degrades gracefully to the global default
/// rather than erroring (see `EditorHostImpl::get_option`).
pub(crate) fn get_option(ctx: &mut SteelCtx, key: String, bid: SteelVal) -> SteelResult {
    let bid = match optional_bid_arg(bid, "get-option")? {
        Some(id) => id,
        None => ctx.focused_buffer_id,
    };
    let value = ctx
        .host
        .settings()
        .get_option(&key, bid)
        .map_err(generic_err)?;
    Ok(match value {
        OptionValue::Bool(b) => SteelVal::BoolV(b),
        OptionValue::Int(n) => SteelVal::IntV(n as isize),
        OptionValue::Str(s) => SteelVal::StringV(s.into()),
    })
}

#[cfg(test)]
mod tests;
