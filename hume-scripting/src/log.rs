use hume_core::Severity;

use crate::builtins::args::{symbol_arg, symbol_enum_arg};

// ── log! builtin ──────────────────────────────────────────────────────────────

/// `(log! severity message)`: push `message` to the pending message buffer.
/// `severity` is a [`Severity`] by its name; any other value raises.
pub(crate) fn log_msg(
    ctx: &mut crate::SteelCtx,
    severity: steel::rvals::SteelVal,
    message: String,
) -> Result<steel::rvals::SteelVal, steel::rerrs::SteelErr> {
    let name = symbol_arg(&severity, "log!", "a severity symbol like 'warning")?;
    let sev = symbol_enum_arg(name, "log!", &Severity::NAMED)?;
    ctx.pending_messages.push((sev, message));
    Ok(steel::rvals::SteelVal::Void)
}

#[cfg(test)]
mod tests;
