/// Log-message severity, defined in the scripting layer so scripting code does
/// not depend on the editor crate.
///
/// The editor maps this to its own `Severity` enum when draining
/// `ScriptingHost::take_pending_messages()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warning,
    Error,
    Trace,
}

// ── log! builtin ──────────────────────────────────────────────────────────────

/// `(log! severity message)` — push `message` to the pending message buffer.
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
        "trace" => LogLevel::Trace,
        "info"  => LogLevel::Info,
        "warn"  => LogLevel::Warning,
        "error" => LogLevel::Error,
        other => steel::stop!(Generic =>
            "log!: unknown severity '{}', expected 'trace, 'info, 'warn, or 'error", other),
    };
    ctx.pending_messages.push((sev, message));
    Ok(steel::rvals::SteelVal::Void)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use steel::rvals::SteelVal;

    use super::*;

    #[test]
    fn log_msg_valid_severity() {
        let mut h = crate::test_support::SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        log_msg(&mut ctx, SteelVal::SymbolV("info".into()), "hello".to_string()).unwrap();
        drop(ctx);
        assert_eq!(h.pending_messages.len(), 1);
        assert_eq!(h.pending_messages[0].1, "hello");
        assert!(matches!(h.pending_messages[0].0, LogLevel::Info));
    }

    #[test]
    fn log_msg_unknown_severity_errors() {
        let mut h = crate::test_support::SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        assert!(log_msg(&mut ctx, SteelVal::SymbolV("bad".into()), "msg".to_string()).is_err());
    }
}
