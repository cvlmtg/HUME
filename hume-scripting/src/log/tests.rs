use steel::rvals::SteelVal;

use super::*;

#[test]
fn log_msg_valid_severity() {
    let mut h = crate::test_support::SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    log_msg(
        &mut ctx,
        SteelVal::SymbolV("info".into()),
        "hello".to_string(),
    )
    .unwrap();
    drop(ctx);
    assert_eq!(h.pending_messages.len(), 1);
    assert_eq!(h.pending_messages[0].1, "hello");
    assert!(matches!(h.pending_messages[0].0, Severity::Info));
}

#[test]
fn log_msg_unknown_severity_errors() {
    let mut h = crate::test_support::SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    assert!(log_msg(&mut ctx, SteelVal::SymbolV("bad".into()), "msg".to_string()).is_err());
}

#[test]
fn log_msg_decodes_each_severity_symbol() {
    let cases = [
        ("trace", Severity::Trace),
        ("info", Severity::Info),
        ("warning", Severity::Warning),
        ("error", Severity::Error),
    ];
    for (symbol, severity) in cases {
        let mut h = crate::test_support::SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        log_msg(&mut ctx, SteelVal::SymbolV(symbol.into()), "m".to_string())
            .unwrap_or_else(|e| panic!("'{symbol}: {e}"));
        drop(ctx);
        assert_eq!(h.pending_messages.len(), 1, "'{symbol}");
        assert_eq!(h.pending_messages[0].0, severity, "'{symbol}");
    }
}
