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
    register_completion_source(
        ctx,
        "src".into(),
        SteelVal::FuncV(dummy_proc),
        sym(target),
        sym(match_kind),
        SteelVal::IntV(priority),
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
