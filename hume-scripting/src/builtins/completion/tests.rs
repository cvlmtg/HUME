use super::*;
use crate::host::PendingTarget;
use crate::test_support::{SteelCtxTestHarness, sym};

fn dummy_proc(_: &[SteelVal]) -> Result<SteelVal, SteelErr> {
    Ok(SteelVal::Void)
}

/// Effects queued so far, in emission order.
fn effects(h: &SteelCtxTestHarness) -> Vec<&Effect> {
    h.effects.iter().map(|e| &e.effect).collect()
}

fn register(ctx: &mut SteelCtx, target: &str, match_kind: &str, priority: isize) -> SteelResult {
    register_with_resolve(ctx, target, match_kind, priority, false)
}

fn register_with_resolve(
    ctx: &mut SteelCtx,
    target: &str,
    match_kind: &str,
    priority: isize,
    resolve: bool,
) -> SteelResult {
    register_full(
        ctx,
        target,
        match_kind,
        priority,
        resolve,
        "",
        SteelVal::BoolV(false),
    )
}

fn register_full(
    ctx: &mut SteelCtx,
    target: &str,
    match_kind: &str,
    priority: isize,
    resolve: bool,
    token_chars: &str,
    cancel: SteelVal,
) -> SteelResult {
    register_completion_source(
        ctx,
        "src".into(),
        crate::Callback::for_test(SteelVal::FuncV(dummy_proc)),
        sym(target),
        sym(match_kind),
        SteelVal::IntV(priority),
        SteelVal::BoolV(resolve),
        SteelVal::StringV(token_chars.into()),
        cancel,
    )
}

/// A `'minibuf` source registered with `cancel` as its `#:cancel`.
fn register_with_cancel(ctx: &mut SteelCtx, cancel: SteelVal) -> SteelResult {
    register_full(ctx, "minibuf", "fuzzy", 0, false, "", cancel)
}

// ── register-completion-source! ───────────────────────────────────────────

/// A valid call queues one `Effect::RegisterCompletionSource` carrying
/// every decoded field; nothing is applied inline.
#[test]
fn register_queues_an_effect_with_the_decoded_fields() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    register(&mut ctx, "buffer", "string", 7).expect("valid registration");
    drop(ctx);
    let effects = effects(&h);
    assert_eq!(effects.len(), 1);
    let Effect::RegisterCompletionSource(reg) = effects[0] else {
        panic!("expected RegisterCompletionSource, got {:?}", effects[0]);
    };
    assert_eq!(reg.name, "src");
    let PendingTarget::Buffer {
        priority, resolve, ..
    } = &reg.target
    else {
        panic!("expected a 'buffer target, got {:?}", reg.target);
    };
    assert_eq!(
        reg.match_kind,
        MatchKind::String {
            case_sensitive: true
        }
    );
    assert_eq!(*priority, 7);
    assert!(!resolve, "#:resolve defaults to #f");
    assert!(reg.cancel.is_none());
}

#[test]
fn register_decodes_a_cancel_procedure() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    register_with_cancel(&mut ctx, SteelVal::FuncV(dummy_proc)).expect("valid registration");
    drop(ctx);
    let Effect::RegisterCompletionSource(reg) = effects(&h)[0] else {
        panic!("expected RegisterCompletionSource");
    };
    assert!(reg.cancel.is_some());
}

#[test]
fn register_rejects_a_cancel_that_is_not_callable() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let err = register_with_cancel(&mut ctx, SteelVal::IntV(1))
        .expect_err("a number is not a procedure")
        .to_string();
    assert!(err.contains("#:cancel"), "got: {err}");
}

/// `#:resolve #t` on a `'buffer` source decodes straight through.
#[test]
fn register_decodes_resolve_true_on_a_buffer_source() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    register_with_resolve(&mut ctx, "buffer", "fuzzy", 0, true).expect("valid registration");
    drop(ctx);
    let Effect::RegisterCompletionSource(reg) = effects(&h)[0] else {
        panic!("expected RegisterCompletionSource");
    };
    assert!(matches!(
        reg.target,
        PendingTarget::Buffer { resolve: true, .. }
    ));
}

/// `#:resolve #t` on a `'minibuf` source is a caller error, not a silently
/// dropped claim: only a `'buffer` source's items can ever be a wire item
/// from the buffer's own attached server.
#[test]
fn register_rejects_resolve_true_on_a_minibuf_source() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let err = register_with_resolve(&mut ctx, "minibuf", "fuzzy", 0, true)
        .expect_err("resolve is buffer-only")
        .to_string();
    assert!(err.contains("#:resolve"), "got: {err}");
}

/// `#:target 'minibuf` decodes to the minibuffer target.
#[test]
fn register_decodes_a_minibuf_target() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    register(&mut ctx, "minibuf", "delegated", 0).expect("valid registration");
    drop(ctx);
    let Effect::RegisterCompletionSource(reg) = effects(&h)[0] else {
        panic!("expected RegisterCompletionSource");
    };
    assert!(matches!(reg.target, PendingTarget::Minibuf));
    assert_eq!(reg.match_kind, MatchKind::Delegated);
}

#[test]
fn register_rejects_an_unknown_target() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let err = register(&mut ctx, "statusline", "fuzzy", 0)
        .expect_err("unknown target")
        .to_string();
    assert!(
        err.contains("'buffer or 'minibuf"),
        "lists the choices; got: {err}"
    );
}

#[test]
fn register_rejects_a_non_callable_proc() {
    let mut host = crate::ScriptingHost::new(&hume_platform::dirs::Dirs::none());
    let mut store = crate::null_host::CommandStoreHost::default();
    let err = host
        .eval_source(
            r#"(register-completion-source! "src" "not a proc" #:target 'buffer)"#,
            &mut store,
        )
        .expect_err("a string is not callable");
    assert!(
        err.contains("register-completion-source!: expected a callable"),
        "got: {err}"
    );
}

// ── set-completion-triggers! ─────────────────────────────────────────

/// Queues an `Effect::SetTriggers` with the decoded fields,
/// nothing applied inline, same shape as `register-completion-source!`'s
/// own test above. This is what lets a same-eval
/// `register-completion-source!` + `set-completion-triggers!` pair
/// apply in emission order instead of racing the registration.
#[test]
fn set_trigger_chars_queues_an_effect_with_the_decoded_fields() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    set_completion_triggers(
        &mut ctx,
        SteelVal::StringV("rust".into()),
        "src".to_string(),
        SteelVal::ListV(vec![SteelVal::CharV('.'), SteelVal::CharV(':')].into()),
    )
    .expect("valid call");
    drop(ctx);
    let effects = effects(&h);
    assert_eq!(effects.len(), 1);
    let Effect::SetTriggers {
        listener,
        scope,
        chars,
    } = effects[0]
    else {
        panic!("expected SetTriggers, got {:?}", effects[0]);
    };
    assert_eq!(
        listener,
        &hume_core::TriggerListener::Completion("src".into())
    );
    assert_eq!(
        scope,
        &crate::types::TriggerScope::Language("rust".to_string())
    );
    assert_eq!(chars, &['.', ':']);
}

#[test]
fn set_attachment_completion_triggers_queues_an_attachment_scope() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let server = crate::ServerRef {
        id: hume_lsp::backend::ServerId(4),
        name: hume_core::ServerName::parse("ra-lint").unwrap(),
    };
    let pane = crate::test_support::default_pane();
    set_attachment_completion_triggers(
        &mut ctx,
        pane,
        "src".to_string(),
        server.into_steel_val(),
        SteelVal::SymbolV("completion".into()),
        SteelVal::ListV(vec![SteelVal::CharV('.')].into()),
    )
    .expect("valid call");
    drop(ctx);
    let effects = effects(&h);
    let Effect::SetTriggers { scope, .. } = effects[0] else {
        panic!("expected SetTriggers, got {:?}", effects[0]);
    };
    assert_eq!(
        scope,
        &crate::types::TriggerScope::Attachment {
            buffer: pane.buffer(),
            server: hume_lsp::backend::ServerId(4),
            feature: hume_core::LspFeature::Completion,
        }
    );
}

#[test]
fn a_language_that_is_not_a_string_raises() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let err = set_completion_triggers(
        &mut ctx,
        SteelVal::IntV(3),
        "src".to_string(),
        SteelVal::ListV(vec![SteelVal::CharV('.')].into()),
    )
    .unwrap_err();
    assert!(err.to_string().contains("expected a string"), "{err}");
}

#[test]
fn an_attachment_server_that_is_not_a_server_raises() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let err = set_attachment_hook_triggers(
        &mut ctx,
        crate::test_support::default_pane(),
        "src".to_string(),
        SteelVal::StringV("rust".into()),
        SteelVal::SymbolV("completion".into()),
        SteelVal::ListV(vec![SteelVal::CharV('.')].into()),
    )
    .unwrap_err();
    assert!(err.to_string().contains("expected a server value"), "{err}");
}

#[test]
fn an_attachment_feature_that_is_not_a_feature_raises() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let server = crate::ServerRef {
        id: hume_lsp::backend::ServerId(4),
        name: hume_core::ServerName::parse("ra-lint").unwrap(),
    };
    let err = set_attachment_hook_triggers(
        &mut ctx,
        crate::test_support::default_pane(),
        "src".to_string(),
        server.into_steel_val(),
        SteelVal::SymbolV("nonsense".into()),
        SteelVal::ListV(vec![SteelVal::CharV('.')].into()),
    )
    .unwrap_err();
    assert!(err.to_string().contains("nonsense"), "{err}");
}

// ── completion-emit! ──────────────────────────────────────────────────────

/// `#:token-chars` decodes straight through on a `'buffer` source and
/// defaults to empty.
#[test]
fn register_decodes_token_chars_on_a_buffer_source() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    register_full(
        &mut ctx,
        "buffer",
        "fuzzy",
        0,
        false,
        "-$",
        SteelVal::BoolV(false),
    )
    .expect("valid registration");
    register(&mut ctx, "buffer", "fuzzy", 0).expect("valid registration");
    drop(ctx);
    let effects = effects(&h);
    let (Effect::RegisterCompletionSource(with), Effect::RegisterCompletionSource(without)) =
        (effects[0], effects[1])
    else {
        panic!("expected two RegisterCompletionSource effects");
    };
    let token_chars = |target: &PendingTarget| match target {
        PendingTarget::Buffer { token_chars, .. } => token_chars.clone(),
        PendingTarget::Minibuf => panic!("expected a 'buffer target"),
    };
    assert_eq!(token_chars(&with.target), "-$");
    assert_eq!(token_chars(&without.target), "");
}

/// A whitespace token char would leave a token with no terminator, same as
/// `word-chars`.
#[test]
fn register_rejects_whitespace_token_chars() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let err = register_full(
        &mut ctx,
        "buffer",
        "fuzzy",
        0,
        false,
        "- ",
        SteelVal::BoolV(false),
    )
    .expect_err("whitespace is not a token char")
    .to_string();
    assert!(err.contains("#:token-chars"), "got: {err}");
}

/// The token is a buffer notion: a `'minibuf` source's token is its own
/// argument span.
#[test]
fn register_rejects_token_chars_on_a_minibuf_source() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let err = register_full(
        &mut ctx,
        "minibuf",
        "fuzzy",
        0,
        false,
        "-",
        SteelVal::BoolV(false),
    )
    .expect_err("token-chars is buffer-only")
    .to_string();
    assert!(err.contains("#:token-chars"), "got: {err}");
}

/// A `'minibuf` session runs one source, so there is no other source for a
/// priority to rank against.
#[test]
fn register_rejects_priority_on_a_minibuf_source() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let err = register(&mut ctx, "minibuf", "fuzzy", 5)
        .expect_err("priority is buffer-only")
        .to_string();
    assert!(
        err.contains("#:priority: only a 'buffer source"),
        "got: {err}"
    );
}

#[test]
fn set_hook_triggers_queues_a_hook_kind_effect() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    set_hook_triggers(
        &mut ctx,
        SteelVal::StringV("rust".into()),
        "sig".to_string(),
        SteelVal::ListV(vec![SteelVal::CharV('(')].into()),
    )
    .expect("valid call");
    drop(ctx);
    let effects = effects(&h);
    let Effect::SetTriggers {
        listener,
        scope,
        chars,
    } = effects[0]
    else {
        panic!("expected SetTriggers, got {:?}", effects[0]);
    };
    assert_eq!(listener, &hume_core::TriggerListener::Hook("sig".into()));
    assert_eq!(
        scope,
        &crate::types::TriggerScope::Language("rust".to_string())
    );
    assert_eq!(chars, &['(']);
}

/// `completion-top` raises on a host with no completion capability (a
/// drawer row renderer's), where `'()` would read as "no candidates".
#[test]
fn completion_top_without_completion_host_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let msg = completion_top(&mut ctx, SteelVal::IntV(5))
        .unwrap_err()
        .to_string();
    assert!(msg.contains("not supported by this host"), "got: {msg}");
}
