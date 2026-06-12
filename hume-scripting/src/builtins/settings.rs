//! `(set-option! key value)` builtin.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;

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
/// Only valid during `init.scm` or plugin load (`is_init = true`); raises a
/// Steel error if called from a command body.
pub(crate) fn set_option(ctx: &mut SteelCtx, key: String, value: SteelVal) -> SteelResult {
    if !ctx.is_init && ctx.plugin_stack.is_empty() {
        steel::stop!(Generic =>
            "set-option!: only valid during init.scm or plugin load, not from a Steel command body");
    }

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
        .set_global_option(&key, &value_str)
        .map_err(|e| steel::rerrs::SteelErr::new(steel::rerrs::ErrorKind::Generic, e))?;

    Ok(SteelVal::Void)
}
