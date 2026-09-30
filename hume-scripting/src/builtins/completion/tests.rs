use super::*;
use crate::host::CompletionSourceTarget;
use crate::test_support::SteelCtxTestHarness;

fn sym(s: &str) -> SteelVal {
    SteelVal::SymbolV(s.into())
}

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
    register_full(ctx, target, match_kind, priority, resolve, "")
}

fn register_full(
    ctx: &mut SteelCtx,
    target: &str,
    match_kind: &str,
    priority: isize,
    resolve: bool,
    token_chars: &str,
) -> SteelResult {
    register_completion_source(
        ctx,
        "src".into(),
        SteelVal::FuncV(dummy_proc),
        sym(target),
        sym(match_kind),
        SteelVal::IntV(priority),
        SteelVal::BoolV(resolve),
        SteelVal::StringV(token_chars.into()),
    )
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
    assert_eq!(reg.target, CompletionSourceTarget::Buffer);
    assert_eq!(
        reg.match_kind,
        MatchKind::String {
            case_sensitive: true
        }
    );
    assert_eq!(reg.priority, 7);
    assert!(!reg.resolve, "#:resolve defaults to #f");
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
    assert!(reg.resolve);
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
    assert_eq!(reg.target, CompletionSourceTarget::Minibuf);
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
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let err = register_completion_source(
        &mut ctx,
        "src".into(),
        SteelVal::StringV("not a proc".into()),
        sym("buffer"),
        sym("fuzzy"),
        SteelVal::IntV(0),
        SteelVal::BoolV(false),
        SteelVal::StringV("".into()),
    )
    .expect_err("a string is not callable")
    .to_string();
    assert!(err.contains("callable"), "got: {err}");
}

/// `register-completion-source!` is `config`-gated in `builtins!`'s
/// registration table. The gate lives in the registration wrapper, so
/// this tests the gate primitive directly, same as `bind-key!`'s tests.
#[test]
fn register_is_blocked_in_command_mode() {
    let mut h = SteelCtxTestHarness::new();
    let result = super::super::errors::require_config(&h.ctx(), "%register-completion-source!");
    let msg = result.expect_err("must error in command mode").to_string();
    assert!(
        msg.contains("register-completion-source!") && !msg.contains('%'),
        "names the friendly wrapper, not the primitive; got: {msg}"
    );
}

// ── set-completion-triggers! ─────────────────────────────────────────

/// Queues an `Effect::SetCompletionTriggers` with the decoded fields,
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
        SteelVal::StringV("src".into()),
        SteelVal::StringV("rust".into()),
        SteelVal::ListV(vec![SteelVal::StringV(".".into()), SteelVal::StringV(":".into())].into()),
    )
    .expect("valid call");
    drop(ctx);
    let effects = effects(&h);
    assert_eq!(effects.len(), 1);
    let Effect::SetCompletionTriggers {
        source,
        language,
        chars,
    } = effects[0]
    else {
        panic!("expected SetCompletionTriggers, got {:?}", effects[0]);
    };
    assert_eq!(source, "src");
    assert_eq!(language, "rust");
    assert_eq!(chars, &['.', ':']);
}

// ── completion-emit! ──────────────────────────────────────────────────────

/// `completion-emit!` is `cmd`-gated: an answer can't land from an
/// init-time eval, where there is no session to answer.
#[test]
fn emit_is_blocked_during_init() {
    let mut h = SteelCtxTestHarness::new();
    let result = super::super::errors::require_cmd(&h.ctx_init(), "%completion-emit!");
    let msg = result.expect_err("must error during init").to_string();
    assert!(msg.contains("completion-emit!"), "got: {msg}");
}

/// `#:token-chars` decodes straight through on a `'buffer` source and
/// defaults to empty.
#[test]
fn register_decodes_token_chars_on_a_buffer_source() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    register_full(&mut ctx, "buffer", "fuzzy", 0, false, "-$").expect("valid registration");
    register(&mut ctx, "buffer", "fuzzy", 0).expect("valid registration");
    drop(ctx);
    let effects = effects(&h);
    let (Effect::RegisterCompletionSource(with), Effect::RegisterCompletionSource(without)) =
        (effects[0], effects[1])
    else {
        panic!("expected two RegisterCompletionSource effects");
    };
    assert_eq!(with.token_chars, "-$");
    assert_eq!(without.token_chars, "");
}

/// A whitespace token char would leave a token with no terminator, same as
/// `word-chars`.
#[test]
fn register_rejects_whitespace_token_chars() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let err = register_full(&mut ctx, "buffer", "fuzzy", 0, false, "- ")
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
    let err = register_full(&mut ctx, "minibuf", "fuzzy", 0, false, "-")
        .expect_err("token-chars is buffer-only")
        .to_string();
    assert!(err.contains("#:token-chars"), "got: {err}");
}
