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

fn register(
    ctx: &mut SteelCtx,
    target: &str,
    token: &str,
    match_kind: &str,
    priority: isize,
) -> SteelResult {
    register_completion_source(
        ctx,
        "src".into(),
        SteelVal::FuncV(dummy_proc),
        sym(target),
        sym(token),
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
    register(&mut ctx, "buffer", "word", "string", 7).expect("valid registration");
    drop(ctx);
    let effects = effects(&h);
    assert_eq!(effects.len(), 1);
    let Effect::RegisterCompletionSource(reg) = effects[0] else {
        panic!("expected RegisterCompletionSource, got {:?}", effects[0]);
    };
    assert_eq!(reg.name, "src");
    assert_eq!(
        reg.target,
        CompletionSourceTarget::Buffer(BufferToken::Word)
    );
    assert_eq!(
        reg.match_kind,
        MatchKind::String {
            case_sensitive: true
        }
    );
    assert_eq!(reg.priority, 7);
}

/// `#:target 'minibuf` takes the minibuffer token vocabulary.
#[test]
fn register_decodes_a_minibuf_target() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    register(&mut ctx, "minibuf", "custom", "delegated", 0).expect("valid registration");
    drop(ctx);
    let Effect::RegisterCompletionSource(reg) = effects(&h)[0] else {
        panic!("expected RegisterCompletionSource");
    };
    assert_eq!(
        reg.target,
        CompletionSourceTarget::Minibuf(MinibufToken::Custom)
    );
    assert_eq!(reg.match_kind, MatchKind::Delegated);
}

/// A token from the other target's vocabulary is rejected by name, before
/// anything is queued — `'arg` belongs to `'minibuf`, `'word` to `'buffer`.
#[test]
fn register_rejects_a_token_from_the_wrong_targets_vocabulary() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let err = register(&mut ctx, "buffer", "arg", "fuzzy", 0)
        .expect_err("'arg is not a buffer token")
        .to_string();
    assert!(err.contains("#:token"), "names the keyword; got: {err}");
    assert!(err.contains("'buffer"), "names the target; got: {err}");
    let err = register(&mut ctx, "minibuf", "word", "fuzzy", 0)
        .expect_err("'word is not a minibuf token")
        .to_string();
    assert!(err.contains("'minibuf"), "names the target; got: {err}");
    drop(ctx);
    assert!(
        effects(&h).is_empty(),
        "validation must reject before queueing anything"
    );
}

#[test]
fn register_rejects_an_unknown_target() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let err = register(&mut ctx, "statusline", "word", "fuzzy", 0)
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
        sym("word"),
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

/// `#:span` decodes a dotted pair of char offsets; `#f` is "none".
#[test]
fn emit_span_decodes_a_dotted_pair_or_false() {
    let pair = crate::builtins::args::cons_pair(SteelVal::IntV(3), SteelVal::IntV(8)).unwrap();
    let decoded = optional_pair_fields(pair, "completion-emit! #:span", "(start . end)")
        .unwrap()
        .map(|(s, e)| (usize_arg(s, "").unwrap(), usize_arg(e, "").unwrap()));
    assert_eq!(decoded, Some((3, 8)));
    let none = optional_pair_fields(
        SteelVal::BoolV(false),
        "completion-emit! #:span",
        "(start . end)",
    )
    .unwrap();
    assert!(none.is_none());
}

/// A proper list is not a span — the wire shape is a pair.
#[test]
fn emit_span_rejects_a_list() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let list = SteelVal::ListV(vec![SteelVal::IntV(3), SteelVal::IntV(8)].into());
    let err = completion_emit(
        &mut ctx,
        SteelVal::IntV(1),
        SteelVal::ListV(vec![].into()),
        SteelVal::BoolV(false),
        list,
    )
    .expect_err("a list is not a pair")
    .to_string();
    assert!(err.contains("(start . end)"), "got: {err}");
}

/// `completion-emit!` is `cmd`-gated: an answer can't land from an
/// init-time eval, where there is no session to answer.
#[test]
fn emit_is_blocked_during_init() {
    let mut h = SteelCtxTestHarness::new();
    let result = super::super::errors::require_cmd(&h.ctx_init(), "%completion-emit!");
    let msg = result.expect_err("must error during init").to_string();
    assert!(msg.contains("completion-emit!"), "got: {msg}");
}
