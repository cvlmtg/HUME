use super::*;
use crate::test_support::SteelCtxTestHarness;
use hume_engine::pipeline::BufferId;
use steel::HashMap as SteelHashMap;
use steel::gc::Gc;
use steel::rvals::IntoSteelVal;

fn list_of(items: &[&str]) -> SteelVal {
    items
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
        .into_steelval()
        .unwrap()
}

/// Builds a Steel hashmap `SteelVal` from `(symbol-key, value)` pairs — the
/// `(hash 'k v ...)` shape `wire_position` decodes.
fn hashmap(entries: Vec<(&str, SteelVal)>) -> SteelVal {
    let mut hm = SteelHashMap::new();
    for (k, v) in entries {
        hm.insert(SteelVal::SymbolV(k.into()), v);
    }
    SteelVal::HashMapV(Gc::new(hm).into())
}

/// A well-formed wire `{"line" "character"}` hashmap.
fn wire_pos(line: isize, character: isize) -> SteelVal {
    hashmap(vec![
        ("line", SteelVal::IntV(line)),
        ("character", SteelVal::IntV(character)),
    ])
}

/// `Effect::LspServerOp` entries queued so far, in emission order.
fn lsp_server_ops(h: &SteelCtxTestHarness) -> Vec<&PendingLspServerOp> {
    h.effects
        .iter()
        .filter_map(|e| match &e.effect {
            Effect::LspServerOp(op) => Some(op),
            _ => None,
        })
        .collect()
}

/// `Effect::LspRequest` entries queued so far on a live `ctx`, in
/// emission order — `ctx.effects` (not the harness) since these tests
/// read before `ctx` drops.
fn lsp_requests<'a>(ctx: &'a SteelCtx) -> Vec<&'a PendingLspRequest> {
    ctx.effects
        .iter()
        .filter_map(|e| match &e.effect {
            Effect::LspRequest(req) => Some(req),
            _ => None,
        })
        .collect()
}

/// Unwraps the single queued op as a `Register`, panicking with a message
/// naming the actual variant otherwise — so a misrouted `Unregister`
/// fails loudly instead of silently indexing the wrong data.
fn expect_register(h: &SteelCtxTestHarness) -> &crate::PendingLspServerReg {
    let ops = lsp_server_ops(h);
    assert_eq!(ops.len(), 1);
    match ops[0] {
        PendingLspServerOp::Register(reg) => reg,
        other => panic!("expected Register, got {other:?}"),
    }
}

#[test]
fn queues_a_pending_registration_in_init_mode() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let result = register_lsp_server(
        &mut ctx,
        "rust".into_steelval().unwrap(),
        "rust-analyzer".into_steelval().unwrap(),
        list_of(&[]),
        list_of(&["Cargo.toml"]),
        SteelVal::BoolV(false),
        SteelVal::BoolV(false),
        list_of(&[]),
    );
    assert!(result.is_ok());
    drop(ctx);
    let reg = expect_register(&h);
    assert_eq!(reg.language, "rust");
    assert_eq!(reg.command, "rust-analyzer");
    assert_eq!(reg.root_markers, vec!["Cargo.toml".to_string()]);
    assert_eq!(reg.init_options, None);
    assert_eq!(reg.settings, None);
    assert_eq!(reg.env, Vec::<(String, String)>::new());
}

/// `#:env` decodes a list of `("KEY" . "VALUE")` dotted pairs into
/// `PendingLspServerReg.env` — the wire shape `steel-server/plugin.scm`
/// uses to point `STEEL_LSP_HOME` at the generated host-globals file.
#[test]
fn decodes_env_dotted_pairs() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let env_val = crate::builtins::args::cons_pair(
        "STEEL_LSP_HOME".into_steelval().unwrap(),
        "/tmp/lsp-home".into_steelval().unwrap(),
    )
    .unwrap();
    let env_list: SteelVal = vec![env_val].into_steelval().unwrap();
    let result = register_lsp_server(
        &mut ctx,
        "scheme".into_steelval().unwrap(),
        "steel-language-server".into_steelval().unwrap(),
        list_of(&[]),
        list_of(&[]),
        SteelVal::BoolV(false),
        SteelVal::BoolV(false),
        env_list,
    );
    assert!(result.is_ok());
    drop(ctx);
    let reg = expect_register(&h);
    assert_eq!(
        reg.env,
        vec![("STEEL_LSP_HOME".to_string(), "/tmp/lsp-home".to_string())]
    );
}

#[test]
fn decodes_steel_hashmap_blobs_to_json() {
    use steel::HashMap as SteelHashMap;
    use steel::gc::Gc;

    let mut init_opts = SteelHashMap::new();
    init_opts.insert(SteelVal::StringV("a".into()), SteelVal::IntV(1));
    let mut settings = SteelHashMap::new();
    settings.insert(SteelVal::StringV("b".into()), SteelVal::IntV(2));

    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let result = register_lsp_server(
        &mut ctx,
        "rust".into_steelval().unwrap(),
        "rust-analyzer".into_steelval().unwrap(),
        list_of(&[]),
        list_of(&[]),
        SteelVal::HashMapV(Gc::new(init_opts).into()),
        SteelVal::HashMapV(Gc::new(settings).into()),
        list_of(&[]),
    );
    assert!(result.is_ok());
    drop(ctx);
    let reg = expect_register(&h);
    assert_eq!(reg.init_options, Some(serde_json::json!({"a": 1})));
    assert_eq!(reg.settings, Some(serde_json::json!({"b": 2})));
}

/// `register-lsp-server!` must queue successfully from a plain
/// command-mode context, not just init/activation, so that
/// `:lsp-install`'s runtime registration path works.
#[test]
fn queues_a_pending_registration_from_command_mode() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = register_lsp_server(
        &mut ctx,
        "rust".into_steelval().unwrap(),
        "rust-analyzer".into_steelval().unwrap(),
        list_of(&[]),
        list_of(&[]),
        SteelVal::BoolV(false),
        SteelVal::BoolV(false),
        list_of(&[]),
    );
    assert!(result.is_ok());
    drop(ctx);
    let reg = expect_register(&h);
    assert_eq!(reg.language, "rust");
}

#[test]
fn allowed_during_plugin_activation_even_though_is_init_is_false() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_activation();
    ctx.plugin_stack
        .push(crate::attribution::PluginId::Core("lsp".to_string()));
    let result = register_lsp_server(
        &mut ctx,
        "rust".into_steelval().unwrap(),
        "rust-analyzer".into_steelval().unwrap(),
        list_of(&[]),
        list_of(&[]),
        SteelVal::BoolV(false),
        SteelVal::BoolV(false),
        list_of(&[]),
    );
    assert!(result.is_ok());
}

#[test]
fn unconvertible_init_options_is_a_type_mismatch_error() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let result = register_lsp_server(
        &mut ctx,
        "rust".into_steelval().unwrap(),
        "rust-analyzer".into_steelval().unwrap(),
        list_of(&[]),
        list_of(&[]),
        SteelVal::FuncV(|_| unreachable!()),
        SteelVal::BoolV(false),
        list_of(&[]),
    );
    assert!(result.is_err());
}

// ── unregister-lsp-server! ────────────────────────────────────────────────

#[test]
fn unregister_queues_an_unregister_op() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = unregister_lsp_server(&mut ctx, "rust".into_steelval().unwrap());
    assert!(result.is_ok());
    drop(ctx);
    let ops = lsp_server_ops(&h);
    assert_eq!(ops.len(), 1);
    match ops[0] {
        PendingLspServerOp::Unregister { language } => assert_eq!(language, "rust"),
        other => panic!("expected Unregister, got {other:?}"),
    }
}

#[test]
fn unregister_is_callable_from_init_mode_too() {
    // Symmetric with register-lsp-server!: neither is gated to a single
    // eval kind.
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let result = unregister_lsp_server(&mut ctx, "rust".into_steelval().unwrap());
    assert!(result.is_ok());
}

/// A reinstall eval emits unregister-then-register; the queue must
/// preserve that order so the apply side sees "tear down the old
/// registration, then install the new one" — not the reverse.
#[test]
fn register_unregister_register_ordering_is_preserved() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    register_lsp_server(
        &mut ctx,
        "rust".into_steelval().unwrap(),
        "rust-analyzer".into_steelval().unwrap(),
        list_of(&[]),
        list_of(&[]),
        SteelVal::BoolV(false),
        SteelVal::BoolV(false),
        list_of(&[]),
    )
    .unwrap();
    unregister_lsp_server(&mut ctx, "rust".into_steelval().unwrap()).unwrap();
    register_lsp_server(
        &mut ctx,
        "rust".into_steelval().unwrap(),
        "rust-analyzer".into_steelval().unwrap(),
        list_of(&["--new-flag"]),
        list_of(&[]),
        SteelVal::BoolV(false),
        SteelVal::BoolV(false),
        list_of(&[]),
    )
    .unwrap();
    drop(ctx);

    let ops = lsp_server_ops(&h);
    assert_eq!(ops.len(), 3);
    assert!(matches!(
        ops[0],
        PendingLspServerOp::Register(reg) if reg.args.is_empty()
    ));
    assert!(matches!(
        ops[1],
        PendingLspServerOp::Unregister { language } if language == "rust"
    ));
    assert!(matches!(
        ops[2],
        PendingLspServerOp::Register(reg) if reg.args == vec!["--new-flag".to_string()]
    ));
}

// ── lsp-stop! / lsp-restart! / lsp-show-status! ──────────────────────────

#[test]
fn lsp_stop_queues_a_stop_op_with_the_given_language() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = lsp_stop(&mut ctx, "rust".into_steelval().unwrap());
    assert!(result.is_ok());
    drop(ctx);
    let ops = lsp_server_ops(&h);
    assert_eq!(ops.len(), 1);
    match ops[0] {
        PendingLspServerOp::Stop { language } => assert_eq!(language.as_deref(), Some("rust")),
        other => panic!("expected Stop, got {other:?}"),
    }
}

#[test]
fn lsp_stop_with_false_arg_queues_no_language() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    lsp_stop(&mut ctx, SteelVal::BoolV(false)).unwrap();
    drop(ctx);
    match lsp_server_ops(&h)[0] {
        PendingLspServerOp::Stop { language } => assert_eq!(*language, None),
        other => panic!("expected Stop, got {other:?}"),
    }
}

#[test]
fn lsp_stop_rejects_init_context() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    ctx.session = crate::context::EvalSession::Init;
    let err = super::super::errors::require_cmd(&ctx, "lsp-stop!").unwrap_err();
    assert!(
        err.to_string().contains("not available during init"),
        "got: {err}"
    );
}

#[test]
fn lsp_restart_queues_a_restart_op_with_the_given_language() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = lsp_restart(&mut ctx, "rust".into_steelval().unwrap());
    assert!(result.is_ok());
    drop(ctx);
    let ops = lsp_server_ops(&h);
    assert_eq!(ops.len(), 1);
    match ops[0] {
        PendingLspServerOp::Restart { language } => {
            assert_eq!(language.as_deref(), Some("rust"))
        }
        other => panic!("expected Restart, got {other:?}"),
    }
}

#[test]
fn lsp_restart_rejects_init_context() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    ctx.session = crate::context::EvalSession::Init;
    let err = super::super::errors::require_cmd(&ctx, "lsp-restart!").unwrap_err();
    assert!(
        err.to_string().contains("not available during init"),
        "got: {err}"
    );
}

#[test]
fn lsp_show_status_queues_a_show_status_op() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = lsp_show_status(&mut ctx);
    assert!(result.is_ok());
    drop(ctx);
    let ops = lsp_server_ops(&h);
    assert_eq!(ops.len(), 1);
    assert!(matches!(ops[0], PendingLspServerOp::ShowStatus));
}

#[test]
fn lsp_show_status_rejects_init_context() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    ctx.session = crate::context::EvalSession::Init;
    let err = super::super::errors::require_cmd(&ctx, "lsp-show-status!").unwrap_err();
    assert!(
        err.to_string().contains("not available during init"),
        "got: {err}"
    );
}

/// Unlike the buffer/pane-touching LSP builtins above,
/// `lsp-registered-for-language?` is a pure registry read and must stay
/// callable during init — `core:lsp`'s load-time scan
/// (`registration.scm`) calls it directly to skip already-registered
/// languages, with no `with-handler` fallback to catch a gate error.
///
/// Fail oracle: change `lsp-registered-for-language?`'s table entry from
/// `open` to `cmd` → this returns `Err` instead of `Ok`.
#[test]
fn lsp_registered_for_language_is_callable_during_init() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    ctx.session = crate::context::EvalSession::Init;
    let result = lsp_registered_for_language(&mut ctx, "rust".into_steelval().unwrap());
    assert_eq!(
        result.unwrap(),
        SteelVal::BoolV(false),
        "NullHost reports nothing registered"
    );
}

fn pending_register(language: &str) -> Effect {
    Effect::LspServerOp(PendingLspServerOp::Register(crate::PendingLspServerReg {
        language: language.to_string(),
        command: "rust-analyzer".to_string(),
        args: Vec::new(),
        root_markers: Vec::new(),
        init_options: None,
        settings: None,
        env: Vec::new(),
    }))
}

fn pending_unregister(language: &str) -> Effect {
    Effect::LspServerOp(PendingLspServerOp::Unregister {
        language: language.to_string(),
    })
}

/// R1: `lsp-registered-for-language?` reads through the `Effect::LspServerOp`
/// entries queued this eval before falling back to the host — a
/// `Register` queued this eval must be visible immediately, not only
/// after the next drain.
#[test]
fn a_queued_register_reports_true_within_the_same_eval() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    ctx.push_effect(pending_register("rust"));
    let result = lsp_registered_for_language(&mut ctx, "rust".into_steelval().unwrap());
    assert_eq!(result.unwrap(), SteelVal::BoolV(true));
}

/// Queue order, not queue presence, decides the answer — a later
/// `Unregister` overrides an earlier `Register` for the same language,
/// matching `Editor::apply_lsp_server_op`'s own last-wins application
/// order exactly.
#[test]
fn register_then_unregister_in_queue_order_reports_false() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    ctx.push_effect(pending_register("rust"));
    ctx.push_effect(pending_unregister("rust"));
    let result = lsp_registered_for_language(&mut ctx, "rust".into_steelval().unwrap());
    assert_eq!(result.unwrap(), SteelVal::BoolV(false));
}

/// The reverse order: this is exactly the install-path shape —
/// `lsp/install-server!` queues `Unregister` for every seeded language
/// before the post-install rescan queues `Register` behind it.
#[test]
fn unregister_then_register_in_queue_order_reports_true() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    ctx.push_effect(pending_unregister("rust"));
    ctx.push_effect(pending_register("rust"));
    let result = lsp_registered_for_language(&mut ctx, "rust".into_steelval().unwrap());
    assert_eq!(result.unwrap(), SteelVal::BoolV(true));
}

/// A queued op for a *different* language must not affect the answer.
#[test]
fn a_queued_op_for_a_different_language_does_not_flip_the_answer() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    ctx.push_effect(pending_register("python"));
    let result = lsp_registered_for_language(&mut ctx, "rust".into_steelval().unwrap());
    assert_eq!(
        result.unwrap(),
        SteelVal::BoolV(false),
        "NullHost reports rust unregistered, and python's queued op is irrelevant"
    );
}

/// `Stop`/`Restart`/`ShowStatus` never change registration state — only
/// `Register`/`Unregister` may flip the answer.
#[test]
fn a_stop_op_alone_does_not_flip_the_answer() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    ctx.push_effect(Effect::LspServerOp(PendingLspServerOp::Stop {
        language: Some("rust".to_string()),
    }));
    let result = lsp_registered_for_language(&mut ctx, "rust".into_steelval().unwrap());
    assert_eq!(
        result.unwrap(),
        SteelVal::BoolV(false),
        "a Stop op must not be mistaken for an Unregister"
    );
}

#[test]
fn lsp_request_queues_the_supersede_key() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = lsp_request(
        &mut ctx,
        SteelVal::BoolV(false),
        "textDocument/completion".into_steelval().unwrap(),
        list_of(&[]),
        SteelVal::BoolV(false),
        SteelVal::BoolV(false),
        "completion".into_steelval().unwrap(),
    );
    assert!(result.is_ok());
    let requests = lsp_requests(&ctx);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].supersede, Some("completion".to_string()));
}

#[test]
fn lsp_request_with_false_supersede_queues_none() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = lsp_request(
        &mut ctx,
        SteelVal::BoolV(false),
        "textDocument/hover".into_steelval().unwrap(),
        list_of(&[]),
        SteelVal::BoolV(false),
        SteelVal::BoolV(false),
        SteelVal::BoolV(false),
    );
    assert!(result.is_ok());
    let requests = lsp_requests(&ctx);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].supersede, None);
}

#[test]
fn lsp_position_to_offset_without_lsp_host_returns_false() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = lsp_position_to_offset(&mut ctx, BidArg(BufferId::default()), wire_pos(0, 0));
    assert_eq!(result.unwrap(), SteelVal::BoolV(false));
}

#[test]
fn lsp_position_to_offset_errors_on_missing_character() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let position = hashmap(vec![("line", SteelVal::IntV(0))]);
    let result = lsp_position_to_offset(&mut ctx, BidArg(BufferId::default()), position);
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("lsp-position->offset"), "got: {msg}");
}

#[test]
fn lsp_position_to_offset_errors_on_non_numeric_line() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let position = hashmap(vec![
        ("line", SteelVal::StringV("zero".into())),
        ("character", SteelVal::IntV(0)),
    ]);
    let result = lsp_position_to_offset(&mut ctx, BidArg(BufferId::default()), position);
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("lsp-position->offset"), "got: {msg}");
}

#[test]
fn lsp_position_to_offset_errors_on_non_hashmap_arg() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = lsp_position_to_offset(&mut ctx, BidArg(BufferId::default()), SteelVal::IntV(5));
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("lsp-position->offset"), "got: {msg}");
}

#[test]
fn lsp_range_to_offsets_without_lsp_host_returns_false() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let range = hashmap(vec![("start", wire_pos(0, 0)), ("end", wire_pos(0, 3))]);
    let result = lsp_range_to_offsets(&mut ctx, BidArg(BufferId::default()), range);
    assert_eq!(result.unwrap(), SteelVal::BoolV(false));
}

#[test]
fn lsp_range_to_offsets_errors_on_missing_end() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let range = hashmap(vec![("start", wire_pos(0, 0))]);
    let result = lsp_range_to_offsets(&mut ctx, BidArg(BufferId::default()), range);
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("lsp-range->offsets"), "got: {msg}");
}

#[test]
fn lsp_range_to_offsets_errors_on_malformed_start() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let range = hashmap(vec![
        ("start", hashmap(vec![("line", SteelVal::IntV(0))])),
        ("end", wire_pos(0, 3)),
    ]);
    let result = lsp_range_to_offsets(&mut ctx, BidArg(BufferId::default()), range);
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("lsp-range->offsets"), "got: {msg}");
}

/// The raw `[start, end]` shape a `ParameterInformation.label` arrives in.
fn offset_pair(items: &[isize]) -> SteelVal {
    items
        .iter()
        .map(|n| SteelVal::IntV(*n))
        .collect::<Vec<_>>()
        .into_steelval()
        .unwrap()
}

#[test]
fn lsp_label_offsets_to_text_without_lsp_host_returns_false() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = lsp_label_offsets_to_text(
        &mut ctx,
        BidArg(BufferId::default()),
        SteelVal::StringV("fn foo(a: i32)".into()),
        offset_pair(&[7, 13]),
    );
    assert_eq!(result.unwrap(), SteelVal::BoolV(false));
}

#[test]
fn lsp_label_offsets_to_text_errors_on_a_wrong_length_offset_list() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    for offsets in [offset_pair(&[7]), offset_pair(&[7, 13, 20])] {
        let result = lsp_label_offsets_to_text(
            &mut ctx,
            BidArg(BufferId::default()),
            SteelVal::StringV("fn foo(a: i32)".into()),
            offsets,
        );
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("lsp-label-offsets->text"), "got: {msg}");
    }
}

#[test]
fn lsp_label_offsets_to_text_errors_on_a_negative_offset() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = lsp_label_offsets_to_text(
        &mut ctx,
        BidArg(BufferId::default()),
        SteelVal::StringV("fn foo(a: i32)".into()),
        offset_pair(&[-1, 13]),
    );
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("lsp-label-offsets->text"), "got: {msg}");
}

#[test]
fn lsp_label_offsets_to_text_errors_on_a_non_string_label() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = lsp_label_offsets_to_text(
        &mut ctx,
        BidArg(BufferId::default()),
        SteelVal::IntV(5),
        offset_pair(&[7, 13]),
    );
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("lsp-label-offsets->text"), "got: {msg}");
}
