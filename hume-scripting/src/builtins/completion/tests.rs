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
    register_completion_source(
        ctx,
        "src".into(),
        SteelVal::FuncV(dummy_proc),
        sym(target),
        sym(match_kind),
        SteelVal::IntV(priority),
        SteelVal::BoolV(resolve),
    )
}

// ── register-completion-source! ───────────────────────────────────────────

/// A valid call queues one `Effect::RegisterCompletionSource` carrying
/// every decoded field — nothing is applied inline.
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
/// dropped claim — only a `'buffer` source's items can ever be a wire item
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
    )
    .expect_err("a string is not callable")
    .to_string();
    assert!(err.contains("callable"), "got: {err}");
}

/// `register-completion-source!` is `config`-gated in `builtins!`'s
/// registration table — the gate lives in the registration wrapper, so
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
