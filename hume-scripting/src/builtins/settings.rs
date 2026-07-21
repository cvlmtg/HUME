//! `(set-option! key value)` / `(get-option key)` builtins.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::host::OptionValue;

use super::errors::generic_err;

type SteelResult = Result<SteelVal, SteelErr>;

/// `(set-option! key value)`
///
/// Sets the global setting `key` to `value`. The value may be a Steel string,
/// boolean, or integer — it is converted to a string and forwarded to the
/// editor's settings layer.
///
/// Only `Global` scope is supported from scripts. Use `:set buffer …` from the
/// command line to override a setting for the active buffer.
///
/// Valid during `init.scm` or any plugin activation (init or runtime); raises
/// a Steel error if called from a plain command body.
pub(crate) fn set_option(ctx: &mut SteelCtx, key: String, value: SteelVal) -> SteelResult {
    // Accept string, bool, or integer for `value` and convert to the string
    // representation that the settings layer expects.
    let value_str = match &value {
        SteelVal::StringV(s) => s.to_string(),
        SteelVal::BoolV(b) => b.to_string(),
        SteelVal::IntV(n) => n.to_string(),
        _ => steel::stop!(TypeMismatch =>
            "set-option!: second arg (value) must be a string, bool, or integer, got {:?}", value),
    };

    ctx.host
        .settings()
        .set_global_option(&key, &value_str)
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
