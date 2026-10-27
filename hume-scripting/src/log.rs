use hume_core::Severity;

// ── log! builtin ──────────────────────────────────────────────────────────────

/// `(log! severity message)`: push `message` to the pending message buffer.
///
/// `severity` must be one of the symbols `'trace`, `'info`, `'warn`, or
/// `'error`.  Any other value raises a Steel error.
pub(crate) fn log_msg(
    ctx: &mut crate::SteelCtx,
    severity: steel::rvals::SteelVal,
    message: String,
) -> Result<steel::rvals::SteelVal, steel::rerrs::SteelErr> {
    let sev_str = match &severity {
        steel::rvals::SteelVal::SymbolV(s) => s.as_str().to_string(),
        _ => steel::stop!(TypeMismatch =>
            "log!: severity must be a symbol ('trace 'info 'warn 'error), got {:?}", severity),
    };
    let sev = match sev_str.as_str() {
        "trace" => Severity::Trace,
        "info" => Severity::Info,
        "warn" => Severity::Warning,
        "error" => Severity::Error,
        other => steel::stop!(Generic =>
            "log!: unknown severity '{}', expected 'trace, 'info, 'warn, or 'error", other),
    };
    ctx.pending_messages.push((sev, message));
    Ok(steel::rvals::SteelVal::Void)
}

#[cfg(test)]
mod tests;
