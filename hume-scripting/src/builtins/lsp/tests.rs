use super::*;
use crate::json::JsonHandle;
use crate::test_support::{SteelCtxTestHarness, default_bid, default_pane, default_pane_with_pane};
use crate::types::{RequestMode, RequestParams, RouteSpec, WhenUnavailable};
use hume_rope::position_encoding::PositionEncoding;
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

/// Builds a Steel hashmap `SteelVal` from `(symbol-key, value)` pairs, the
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
/// emission order. `ctx.effects` (not the harness) since these tests
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
/// naming the actual variant otherwise, so a misrouted `Unregister`
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
        "rust-analyzer".into_steelval().unwrap(),
        "rust-analyzer".into_steelval().unwrap(),
        list_of(&[]),
        SteelVal::BoolV(false),
        SteelVal::BoolV(false),
        list_of(&[]),
    );
    assert!(result.is_ok());
    drop(ctx);
    let reg = expect_register(&h);
    assert_eq!(reg.name.as_str(), "rust-analyzer");
    assert_eq!(reg.command, "rust-analyzer");
    assert_eq!(reg.init_options, None);
    assert_eq!(reg.settings, None);
    assert_eq!(reg.env, Vec::<(String, String)>::new());
}

/// `#:env` decodes a list of `("KEY" . "VALUE")` dotted pairs into
/// `PendingLspServerReg.env`, the wire shape `steel-server/plugin.scm`
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
        "steel-language-server".into_steelval().unwrap(),
        "steel-language-server".into_steelval().unwrap(),
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
        "rust-analyzer".into_steelval().unwrap(),
        "rust-analyzer".into_steelval().unwrap(),
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
        "rust-analyzer".into_steelval().unwrap(),
        "rust-analyzer".into_steelval().unwrap(),
        list_of(&[]),
        SteelVal::BoolV(false),
        SteelVal::BoolV(false),
        list_of(&[]),
    );
    assert!(result.is_ok());
    drop(ctx);
    let reg = expect_register(&h);
    assert_eq!(reg.name.as_str(), "rust-analyzer");
}

#[test]
fn allowed_during_plugin_activation_even_though_is_init_is_false() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_activation();
    ctx.plugin_stack.push(crate::attribution::EntryId::main(
        crate::attribution::PluginId::Core("lsp".to_string()),
    ));
    let result = register_lsp_server(
        &mut ctx,
        "rust-analyzer".into_steelval().unwrap(),
        "rust-analyzer".into_steelval().unwrap(),
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
        "rust-analyzer".into_steelval().unwrap(),
        "rust-analyzer".into_steelval().unwrap(),
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
    let result = unregister_lsp_server(&mut ctx, "rust-analyzer".into_steelval().unwrap());
    assert!(result.is_ok());
    drop(ctx);
    let ops = lsp_server_ops(&h);
    assert_eq!(ops.len(), 1);
    match ops[0] {
        PendingLspServerOp::Unregister { name } => assert_eq!(name.as_str(), "rust-analyzer"),
        other => panic!("expected Unregister, got {other:?}"),
    }
}

#[test]
fn unregister_is_callable_from_init_mode_too() {
    // Symmetric with register-lsp-server!: neither is gated to a single
    // eval kind.
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let result = unregister_lsp_server(&mut ctx, "rust-analyzer".into_steelval().unwrap());
    assert!(result.is_ok());
}

/// A reinstall eval emits unregister-then-register; the queue must
/// preserve that order so the apply side sees "tear down the old
/// registration, then install the new one", not the reverse.
#[test]
fn register_unregister_register_ordering_is_preserved() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    register_lsp_server(
        &mut ctx,
        "rust-analyzer".into_steelval().unwrap(),
        "rust-analyzer".into_steelval().unwrap(),
        list_of(&[]),
        SteelVal::BoolV(false),
        SteelVal::BoolV(false),
        list_of(&[]),
    )
    .unwrap();
    unregister_lsp_server(&mut ctx, "rust-analyzer".into_steelval().unwrap()).unwrap();
    register_lsp_server(
        &mut ctx,
        "rust-analyzer".into_steelval().unwrap(),
        "rust-analyzer".into_steelval().unwrap(),
        list_of(&["--new-flag"]),
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
        PendingLspServerOp::Unregister { name } if name.as_str() == "rust-analyzer"
    ));
    assert!(matches!(
        ops[2],
        PendingLspServerOp::Register(reg) if reg.args == vec!["--new-flag".to_string()]
    ));
}

// ── lsp-stop! / lsp-restart! / lsp-show-status! ──────────────────────────

fn server_name(s: &str) -> ServerName {
    ServerName::parse(s).unwrap()
}

#[test]
fn lsp_stop_queues_a_stop_op_with_the_given_name() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = lsp_stop(
        &mut ctx,
        LspServerTarget::Name(server_name("rust-analyzer")),
    );
    assert!(result.is_ok());
    drop(ctx);
    let ops = lsp_server_ops(&h);
    assert_eq!(ops.len(), 1);
    match &ops[0] {
        PendingLspServerOp::Stop {
            target: LspServerTarget::Name(name),
        } => assert_eq!(name.as_str(), "rust-analyzer"),
        other => panic!("expected Stop{{Name}}, got {other:?}"),
    }
}

#[test]
fn lsp_stop_queues_a_stop_op_with_the_given_buffer() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let bid = default_bid();
    lsp_stop(&mut ctx, LspServerTarget::Buffer(bid)).unwrap();
    drop(ctx);
    match lsp_server_ops(&h)[0] {
        PendingLspServerOp::Stop {
            target: LspServerTarget::Buffer(target_bid),
        } => assert_eq!(*target_bid, bid),
        other => panic!("expected Stop{{Buffer}}, got {other:?}"),
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
fn lsp_restart_queues_a_restart_op_with_the_given_name() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = lsp_restart(
        &mut ctx,
        LspServerTarget::Name(server_name("rust-analyzer")),
    );
    assert!(result.is_ok());
    drop(ctx);
    let ops = lsp_server_ops(&h);
    assert_eq!(ops.len(), 1);
    match &ops[0] {
        PendingLspServerOp::Restart {
            target: LspServerTarget::Name(name),
        } => assert_eq!(name.as_str(), "rust-analyzer"),
        other => panic!("expected Restart{{Name}}, got {other:?}"),
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
    let result = lsp_show_status(&mut ctx, default_pane());
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
/// `lsp-server-registered?` is a pure registry read and must stay callable
/// during init: `core:lsp-install`'s load-time scan (`register.scm`) calls
/// it directly to skip already-registered names, with no `with-handler`
/// fallback to catch a gate error. Its table entry is `open` for that
/// reason.
#[test]
fn lsp_server_registered_is_callable_during_init() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    ctx.session = crate::context::EvalSession::Init;
    let result = lsp_server_registered(&mut ctx, "rust-analyzer".into_steelval().unwrap());
    assert_eq!(
        result.unwrap(),
        SteelVal::BoolV(false),
        "NullHost reports nothing registered"
    );
}

fn pending_register(name: &str) -> Effect {
    Effect::LspServerOp(PendingLspServerOp::Register(crate::PendingLspServerReg {
        name: server_name(name),
        command: "rust-analyzer".to_string(),
        args: Vec::new(),
        init_options: None,
        settings: None,
        env: Vec::new(),
    }))
}

fn pending_unregister(name: &str) -> Effect {
    Effect::LspServerOp(PendingLspServerOp::Unregister {
        name: server_name(name),
    })
}

fn registered(ctx: &mut SteelCtx, name: &str) -> SteelVal {
    lsp_server_registered(ctx, name.into_steelval().unwrap()).unwrap()
}

/// `lsp-server-registered?` reads through the `Effect::LspServerOp`
/// entries queued this eval before falling back to the host, in queue
/// order: the last `Register`/`Unregister` of the name wins, matching
/// `Editor::apply_lsp_server_ops`'s own application order.
#[test]
fn lsp_server_registered_reads_through_queued_ops() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    ctx.push_effect(pending_register("rust-analyzer"));
    assert_eq!(registered(&mut ctx, "rust-analyzer"), SteelVal::BoolV(true));
    ctx.push_effect(pending_unregister("rust-analyzer"));
    assert_eq!(
        registered(&mut ctx, "rust-analyzer"),
        SteelVal::BoolV(false)
    );
    ctx.push_effect(pending_register("rust-analyzer"));
    assert_eq!(registered(&mut ctx, "rust-analyzer"), SteelVal::BoolV(true));
}

/// A queued op for a *different* name must not affect the answer.
#[test]
fn a_queued_op_for_a_different_name_does_not_flip_the_answer() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    ctx.push_effect(pending_register("pyright"));
    assert_eq!(
        registered(&mut ctx, "rust-analyzer"),
        SteelVal::BoolV(false),
        "NullHost reports rust-analyzer unregistered, and pyright's queued op is irrelevant"
    );
}

/// `Stop`/`Restart`/`ShowStatus` never change registration state; only
/// `Register`/`Unregister` may flip the answer.
#[test]
fn a_stop_op_alone_does_not_flip_the_answer() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    ctx.push_effect(Effect::LspServerOp(PendingLspServerOp::Stop {
        target: LspServerTarget::Name(server_name("rust-analyzer")),
    }));
    assert_eq!(
        registered(&mut ctx, "rust-analyzer"),
        SteelVal::BoolV(false),
        "a Stop op must not be mistaken for an Unregister"
    );
}

// ── register-lsp-server!'s name ──────────────────────────────────────────

fn register_named(ctx: &mut SteelCtx, name: &str) -> SteelResult {
    register_lsp_server(
        ctx,
        name.into_steelval().unwrap(),
        "cmd".into_steelval().unwrap(),
        list_of(&[]),
        SteelVal::BoolV(false),
        SteelVal::BoolV(false),
        list_of(&[]),
    )
}

#[test]
fn register_queues_the_name() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    register_named(&mut ctx, "tsls").unwrap();
    drop(ctx);
    assert_eq!(expect_register(&h).name.as_str(), "tsls");
}

#[test]
fn register_rejects_a_name_with_whitespace() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init();
    let err = register_named(&mut ctx, "rust analyzer").unwrap_err();
    assert!(err.to_string().contains("whitespace"), "{err}");
}

// ── set-language-servers! / set-default-language-servers! ──────────────────

fn symbols(items: &[&str]) -> SteelVal {
    SteelVal::ListV(
        items
            .iter()
            .map(|s| SteelVal::SymbolV((*s).into()))
            .collect::<Vec<_>>()
            .into(),
    )
}

fn set_list(ctx: &mut SteelCtx, entries: SteelVal) -> SteelResult {
    set_language_servers(ctx, "python".into_steelval().unwrap(), entries)
}

fn queued_lists(h: &SteelCtxTestHarness) -> Vec<(ListLayer, Option<Vec<ListEntry>>)> {
    lsp_server_ops(h)
        .into_iter()
        .map(|op| match op {
            PendingLspServerOp::SetLanguageServers {
                language,
                layer,
                entries,
            } => {
                assert_eq!(language, "python");
                (*layer, entries.clone())
            }
            other => panic!("expected SetLanguageServers, got {other:?}"),
        })
        .collect()
}

#[test]
fn set_language_servers_queues_names_and_filters() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let entries = SteelVal::ListV(
        vec![
            "ty".into_steelval().unwrap(),
            hashmap(vec![
                ("name", "ruff".into_steelval().unwrap()),
                ("only-features", symbols(&["format", "diagnostics"])),
            ]),
        ]
        .into(),
    );
    set_list(&mut ctx, entries).unwrap();
    set_default_language_servers(
        &mut ctx,
        "python".into_steelval().unwrap(),
        SteelVal::ListV(vec!["jedi".into_steelval().unwrap()].into()),
    )
    .unwrap();
    drop(ctx);

    assert_eq!(
        queued_lists(&h),
        vec![
            (
                ListLayer::User,
                Some(vec![
                    ListEntry {
                        name: server_name("ty"),
                        filter: FeatureFilter::All,
                    },
                    ListEntry {
                        name: server_name("ruff"),
                        filter: FeatureFilter::Only(
                            [LspFeature::Format, LspFeature::Diagnostics]
                                .into_iter()
                                .collect()
                        ),
                    },
                ])
            ),
            (
                ListLayer::Default,
                Some(vec![ListEntry {
                    name: server_name("jedi"),
                    filter: FeatureFilter::All,
                }])
            ),
        ]
    );
}

#[test]
fn set_language_servers_false_clears_the_layer() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    set_list(&mut ctx, SteelVal::BoolV(false)).unwrap();
    drop(ctx);
    assert_eq!(queued_lists(&h), vec![(ListLayer::User, None)]);
}

#[test]
fn set_language_servers_rejects_a_duplicate_name() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let err = set_list(&mut ctx, list_of(&["ty", "ty"])).unwrap_err();
    assert!(err.to_string().contains("listed twice"), "{err}");
}

#[test]
fn set_language_servers_rejects_both_filters_on_one_entry() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let entry = hashmap(vec![
        ("name", "ruff".into_steelval().unwrap()),
        ("only-features", symbols(&["format"])),
        ("except-features", symbols(&["hover"])),
    ]);
    let err = set_list(&mut ctx, SteelVal::ListV(vec![entry].into())).unwrap_err();
    assert!(err.to_string().contains("both"), "{err}");
}

#[test]
fn set_language_servers_rejects_an_unknown_feature() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let entry = hashmap(vec![
        ("name", "ruff".into_steelval().unwrap()),
        ("only-features", symbols(&["formatting"])),
    ]);
    assert!(set_list(&mut ctx, SteelVal::ListV(vec![entry].into())).is_err());
}

/// `%lsp-request!`'s arguments, each `#f` unless a test sets it.
struct RequestArgs {
    pane: PaneHandle,
    params: SteelVal,
    feature: SteelVal,
    to: SteelVal,
    unavailable: SteelVal,
    supersede: SteelVal,
    require_focus: SteelVal,
    tracked: SteelVal,
}

impl Default for RequestArgs {
    fn default() -> Self {
        Self {
            pane: default_pane(),
            params: hashmap(vec![]),
            feature: SteelVal::BoolV(false),
            to: SteelVal::BoolV(false),
            unavailable: SteelVal::SymbolV("error".into()),
            supersede: SteelVal::BoolV(false),
            require_focus: SteelVal::BoolV(false),
            tracked: SteelVal::BoolV(false),
        }
    }
}

fn request(ctx: &mut SteelCtx, args: RequestArgs) -> SteelResult {
    lsp_request(
        ctx,
        args.pane,
        "textDocument/hover".into_steelval().unwrap(),
        args.params,
        SteelVal::FuncV(noop_proc),
        args.feature,
        args.to,
        args.unavailable,
        SteelVal::BoolV(false),
        args.supersede,
        args.require_focus,
        args.tracked,
    )
}

fn request_all(ctx: &mut SteelCtx, params: SteelVal) -> SteelResult {
    lsp_request_all(
        ctx,
        default_pane(),
        "textDocument/codeAction".into_steelval().unwrap(),
        params,
        SteelVal::FuncV(noop_proc),
        SteelVal::BoolV(false),
        SteelVal::SymbolV("error".into()),
        SteelVal::BoolV(false),
        SteelVal::BoolV(false),
        SteelVal::BoolV(false),
        SteelVal::BoolV(false),
    )
}

fn server(id: u32, name: &str) -> SteelVal {
    crate::ServerRef {
        id: hume_lsp::backend::ServerId(id),
        name: ServerName::parse(name).unwrap(),
    }
    .into_steel_val()
}

#[test]
fn lsp_request_queues_the_supersede_key() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    request(
        &mut ctx,
        RequestArgs {
            supersede: "completion".into_steelval().unwrap(),
            ..RequestArgs::default()
        },
    )
    .expect("a well-formed request queues");
    let requests = lsp_requests(&ctx);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].supersede, Some("completion".to_string()));
}

#[test]
fn lsp_request_queues_the_tracked_token_and_none_for_false() {
    for (tracked, expected) in [
        (SteelVal::IntV(7), Some(crate::host::HostToken::from_raw(7))),
        (SteelVal::BoolV(false), None),
    ] {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        request(
            &mut ctx,
            RequestArgs {
                tracked,
                ..RequestArgs::default()
            },
        )
        .expect("a well-formed request queues");
        assert_eq!(lsp_requests(&ctx)[0].tracked, expected);
    }
}

#[test]
fn lsp_request_with_false_supersede_queues_none() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    request(&mut ctx, RequestArgs::default()).expect("a well-formed request queues");
    let requests = lsp_requests(&ctx);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].supersede, None);
}

#[test]
fn lsp_request_decodes_require_focus() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    request(
        &mut ctx,
        RequestArgs {
            pane: default_pane_with_pane(),
            require_focus: SteelVal::BoolV(true),
            ..RequestArgs::default()
        },
    )
    .expect("a well-formed request queues");
    let requests = lsp_requests(&ctx);
    assert_eq!(requests.len(), 1);
    assert!(requests[0].require_focus.is_some());
}

#[test]
fn lsp_request_decodes_feature_method_and_to() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    request(
        &mut ctx,
        RequestArgs {
            feature: SteelVal::SymbolV("hover".into()),
            to: server(3, "rust-analyzer"),
            ..RequestArgs::default()
        },
    )
    .expect("a well-formed request queues");
    let requests = lsp_requests(&ctx);
    assert_eq!(requests[0].mode, RequestMode::Single);
    assert_eq!(
        requests[0].route,
        RouteSpec {
            feature: Some(LspFeature::Hover),
            to: Some(crate::ServerRef {
                id: hume_lsp::backend::ServerId(3),
                name: ServerName::parse("rust-analyzer").unwrap(),
            }),
        }
    );
}

#[test]
fn lsp_request_decodes_unavailable() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    request(
        &mut ctx,
        RequestArgs {
            unavailable: SteelVal::SymbolV("empty".into()),
            ..RequestArgs::default()
        },
    )
    .expect("a well-formed request queues");
    request(&mut ctx, RequestArgs::default()).expect("a well-formed request queues");
    let requests = lsp_requests(&ctx);
    assert_eq!(requests[0].when_unavailable, WhenUnavailable::Empty);
    assert_eq!(requests[1].when_unavailable, WhenUnavailable::Error);
}

#[test]
fn lsp_request_rejects_an_unknown_unavailable() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let err = request(
        &mut ctx,
        RequestArgs {
            unavailable: SteelVal::SymbolV("quiet".into()),
            ..RequestArgs::default()
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("quiet"), "{err}");
}

#[test]
fn lsp_request_rejects_an_unknown_feature() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let err = request(
        &mut ctx,
        RequestArgs {
            feature: SteelVal::SymbolV("hovering".into()),
            ..RequestArgs::default()
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("'hover"), "{err}");
    assert!(lsp_requests(&ctx).is_empty());
}

#[test]
fn lsp_request_rejects_a_to_that_is_not_a_server() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let err = request(
        &mut ctx,
        RequestArgs {
            to: "rust-analyzer".into_steelval().unwrap(),
            ..RequestArgs::default()
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("#:to"), "{err}");
}

#[test]
fn lsp_request_rejects_params_that_cannot_become_json() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let err = request(
        &mut ctx,
        RequestArgs {
            params: SteelVal::FuncV(noop_proc),
            ..RequestArgs::default()
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("lsp-request! params"), "{err}");
}

#[test]
fn lsp_request_all_decodes_per_server_pairs() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let pairs = SteelVal::ListV(
        vec![
            crate::builtins::args::cons_pair(server(1, "ty"), hashmap(vec![])).unwrap(),
            crate::builtins::args::cons_pair(server(2, "ruff"), hashmap(vec![])).unwrap(),
        ]
        .into(),
    );
    request_all(&mut ctx, pairs).expect("a pair list queues");
    let requests = lsp_requests(&ctx);
    assert_eq!(requests[0].mode, RequestMode::All);
    let RequestParams::PerServer(members) = &requests[0].params else {
        panic!("expected per-server params");
    };
    let ids: Vec<u32> = members.iter().map(|(server, _)| server.id.0).collect();
    assert_eq!(ids, vec![1, 2]);
}

#[test]
fn lsp_request_all_takes_a_hash_as_shared_params() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    request_all(&mut ctx, hashmap(vec![])).expect("a hash queues");
    assert!(matches!(
        lsp_requests(&ctx)[0].params,
        RequestParams::Shared(_)
    ));
}

#[test]
fn lsp_request_all_rejects_a_pair_without_a_server() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let pairs = SteelVal::ListV(
        vec![
            crate::builtins::args::cons_pair("ty".into_steelval().unwrap(), hashmap(vec![]))
                .unwrap(),
        ]
        .into(),
    );
    let err = request_all(&mut ctx, pairs).unwrap_err();
    assert!(err.to_string().contains("(server . params)"), "{err}");
    assert!(lsp_requests(&ctx).is_empty());
}

#[test]
fn lsp_request_all_rejects_an_empty_pair_list() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let err = request_all(&mut ctx, SteelVal::ListV(Vec::<SteelVal>::new().into())).unwrap_err();
    assert!(err.to_string().contains("empty"), "{err}");
    assert!(lsp_requests(&ctx).is_empty());
}

#[test]
fn lsp_request_all_rejects_a_server_listed_twice() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let pairs = SteelVal::ListV(
        vec![
            crate::builtins::args::cons_pair(server(1, "ty"), hashmap(vec![])).unwrap(),
            crate::builtins::args::cons_pair(server(1, "ty"), hashmap(vec![])).unwrap(),
        ]
        .into(),
    );
    let err = request_all(&mut ctx, pairs).unwrap_err();
    assert!(err.to_string().contains("listed twice"), "{err}");
    assert!(lsp_requests(&ctx).is_empty());
}

#[test]
fn position_params_hash_carries_a_doc_pos() {
    let pos = crate::DocPos {
        buffer: default_bid(),
        version: hume_editing::text::BufferText::from("a\n").version(),
        offset: hume_rope::offset::CharOffset::new(4),
    };
    let params = position_params_value(crate::host::PositionParams {
        uri: "file:///a.rs".to_string(),
        pos,
    });
    let SteelVal::HashMapV(map) = params else {
        panic!("expected a hash");
    };
    let position = map
        .get(&"position".into_steelval().unwrap())
        .expect("a position key");
    assert_eq!(crate::DocPos::from_steel_val(position), Some(pos));
    let document = map
        .get(&"textDocument".into_steelval().unwrap())
        .expect("a textDocument key");
    assert_eq!(
        crate::json::steel_to_json(document).unwrap(),
        serde_json::json!({ "uri": "file:///a.rs" })
    );
}

/// A hand-built (untagged) position/range (`wire_pos`/`hashmap`'s own
/// shape) is rejected outright, before this builtin ever reaches its
/// "no LSP host" branch: `JsonHandle::position_encoding` errors on
/// `WireOrigin::Local` first. This is `lsp_position_to_offset_untagged_
/// handle_errors`/`lsp_range_to_offsets_untagged_handle_errors`'s own
/// unit-level counterpart; the two `hume-editor` regression tests exercise
/// the same rule through a real command dispatch.
#[test]
fn lsp_position_to_offset_untagged_handle_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = lsp_position_to_offset(&mut ctx, default_pane(), wire_pos(0, 0));
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("not a value from an LSP server"), "got: {msg}");
}

/// [`lsp_position_to_offset_untagged_handle_errors`]'s "no LSP host"
/// sibling: given a *tagged* handle (so the encoding check passes), a
/// harness with no LSP host still answers `#f`, not an error: the
/// `ctx.host.lsp()` branch, unaffected by where the encoding came from.
#[test]
fn lsp_position_to_offset_without_lsp_host_returns_false() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let position = JsonHandle::server_for_test(
        serde_json::json!({"line": 0, "character": 0}),
        PositionEncoding::Utf16,
    )
    .into_steel_val();
    let result = lsp_position_to_offset(&mut ctx, default_pane(), position);
    assert_eq!(result.unwrap(), SteelVal::BoolV(false));
}

#[test]
fn lsp_position_to_offset_errors_on_missing_character() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let position =
        JsonHandle::server_for_test(serde_json::json!({"line": 0}), PositionEncoding::Utf16)
            .into_steel_val();
    let result = lsp_position_to_offset(&mut ctx, default_pane(), position);
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("lsp-position->offset"), "got: {msg}");
}

#[test]
fn lsp_position_to_offset_errors_on_non_numeric_line() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let position = JsonHandle::server_for_test(
        serde_json::json!({"line": "zero", "character": 0}),
        PositionEncoding::Utf16,
    )
    .into_steel_val();
    let result = lsp_position_to_offset(&mut ctx, default_pane(), position);
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("lsp-position->offset"), "got: {msg}");
}

#[test]
fn lsp_position_to_offset_errors_on_non_hashmap_arg() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = lsp_position_to_offset(&mut ctx, default_pane(), SteelVal::IntV(5));
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("lsp-position->offset"), "got: {msg}");
}

/// [`lsp_position_to_offset_untagged_handle_errors`]'s `lsp-range->offsets`
/// counterpart.
#[test]
fn lsp_range_to_offsets_untagged_handle_errors() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let range = hashmap(vec![("start", wire_pos(0, 0)), ("end", wire_pos(0, 3))]);
    let result = lsp_range_to_offsets(&mut ctx, default_pane(), range);
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("not a value from an LSP server"), "got: {msg}");
}

#[test]
fn lsp_range_to_offsets_without_lsp_host_returns_false() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let range = JsonHandle::server_for_test(
        serde_json::json!({"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 3}}),
        PositionEncoding::Utf16,
    )
    .into_steel_val();
    let result = lsp_range_to_offsets(&mut ctx, default_pane(), range);
    assert_eq!(result.unwrap(), SteelVal::BoolV(false));
}

#[test]
fn lsp_range_to_offsets_errors_on_missing_end() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let range = JsonHandle::server_for_test(
        serde_json::json!({"start": {"line": 0, "character": 0}}),
        PositionEncoding::Utf16,
    )
    .into_steel_val();
    let result = lsp_range_to_offsets(&mut ctx, default_pane(), range);
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("lsp-range->offsets"), "got: {msg}");
}

#[test]
fn lsp_range_to_offsets_errors_on_malformed_start() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let range = JsonHandle::server_for_test(
        serde_json::json!({"start": {"line": 0}, "end": {"line": 0, "character": 3}}),
        PositionEncoding::Utf16,
    )
    .into_steel_val();
    let result = lsp_range_to_offsets(&mut ctx, default_pane(), range);
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("lsp-range->offsets"), "got: {msg}");
}

/// The raw `[start, end]` shape a `ParameterInformation.label` arrives in,
/// always server-tagged in practice (`json-ref`'d straight out of the
/// signature-help response `label` rode in on), so this mints a
/// server-tagged handle rather than a hand-built Steel list.
fn offset_pair(items: &[isize]) -> SteelVal {
    JsonHandle::server_for_test(
        items
            .iter()
            .map(|&n| serde_json::json!(n))
            .collect::<Vec<_>>()
            .into(),
        PositionEncoding::Utf16,
    )
    .into_steel_val()
}

#[test]
fn lsp_label_offsets_to_text_without_lsp_host_returns_false() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = lsp_label_offsets_to_text(
        &mut ctx,
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
    let result = lsp_label_offsets_to_text(&mut ctx, SteelVal::IntV(5), offset_pair(&[7, 13]));
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("lsp-label-offsets->text"), "got: {msg}");
}

/// A hand-built (untagged) offsets value has no producing server to have
/// negotiated an encoding with, so it is rejected before the array-shape check
/// even runs.
#[test]
fn lsp_label_offsets_to_text_rejects_an_untagged_offsets_value() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let untagged: SteelVal = vec![SteelVal::IntV(7), SteelVal::IntV(13)]
        .into_steelval()
        .unwrap();
    let result = lsp_label_offsets_to_text(
        &mut ctx,
        SteelVal::StringV("fn foo(a: i32)".into()),
        untagged,
    );
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("not a value from an LSP server"), "got: {msg}");
}

// ── JsonHandle arguments (json_arg funnel) ─────────────────────────────────────

#[test]
fn lsp_label_offsets_to_text_accepts_a_json_array_handle() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let offsets = JsonHandle::server_for_test(serde_json::json!([7, 13]), PositionEncoding::Utf16)
        .into_steel_val();
    let result = lsp_label_offsets_to_text(
        &mut ctx,
        SteelVal::StringV("fn foo(a: i32)".into()),
        offsets,
    );
    assert_eq!(result.unwrap(), SteelVal::BoolV(false));
}

#[test]
fn lsp_label_offsets_to_text_errors_on_a_wrong_length_array_handle() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let offsets = JsonHandle::server_for_test(serde_json::json!([7]), PositionEncoding::Utf16)
        .into_steel_val();
    let result = lsp_label_offsets_to_text(
        &mut ctx,
        SteelVal::StringV("fn foo(a: i32)".into()),
        offsets,
    );
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("lsp-label-offsets->text"), "got: {msg}");
}

#[test]
fn lsp_locations_to_display_parts_accepts_a_list_of_json_handles() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let locs: SteelVal = vec![
        JsonHandle::new(serde_json::json!({
            "uri": "file:///a.rs",
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
        }))
        .into_steel_val(),
    ]
    .into_steelval()
    .unwrap();
    let msg = lsp_locations_to_display_parts(&mut ctx, locs)
        .unwrap_err()
        .to_string();
    assert!(msg.contains("no LSP state available"), "got: {msg}");
}

// ── register-lsp-notification-hook! ──────────────────────────────────────────

fn noop_proc(_args: &[SteelVal]) -> SteelResult {
    Ok(SteelVal::Void)
}

/// The keys the single registered `on-lsp-notification` entry carries.
fn registered_keys(h: &SteelCtxTestHarness) -> Option<Vec<String>> {
    let entries = h.registries.hooks.handlers_for("on-lsp-notification");
    assert_eq!(entries.len(), 1, "one entry must be registered, no more");
    entries[0].keys.as_ref().map(|k| k.to_vec())
}

#[test]
fn register_lsp_notification_hook_accepts_a_single_method_string() {
    let mut h = SteelCtxTestHarness::new();
    register_lsp_notification_hook(
        &mut h.ctx_init(),
        SteelVal::StringV("a/b".into()),
        SteelVal::FuncV(noop_proc),
    )
    .expect("a method string must register");
    assert_eq!(registered_keys(&h), Some(vec!["a/b".to_string()]));
}

#[test]
fn register_lsp_notification_hook_accepts_a_list_of_methods() {
    let mut h = SteelCtxTestHarness::new();
    register_lsp_notification_hook(
        &mut h.ctx_init(),
        list_of(&["a", "b", "c"]),
        SteelVal::FuncV(noop_proc),
    )
    .expect("a method list must register");
    assert_eq!(
        registered_keys(&h),
        Some(vec!["a".to_string(), "b".to_string(), "c".to_string()])
    );
}

/// An empty filter could never fire, so it is rejected instead of stored.
#[test]
fn register_lsp_notification_hook_rejects_an_empty_method_list() {
    let mut h = SteelCtxTestHarness::new();
    let msg =
        register_lsp_notification_hook(&mut h.ctx_init(), list_of(&[]), SteelVal::FuncV(noop_proc))
            .expect_err("an empty list must be rejected")
            .to_string();
    assert!(msg.contains("must not be empty"), "got: {msg}");
    assert!(!h.registries.hooks.has_match("on-lsp-notification", None));
}

#[test]
fn register_lsp_notification_hook_rejects_a_non_string_method() {
    let mut h = SteelCtxTestHarness::new();
    let result = register_lsp_notification_hook(
        &mut h.ctx_init(),
        vec![SteelVal::IntV(1)].into_steelval().unwrap(),
        SteelVal::FuncV(noop_proc),
    );
    assert!(result.is_err(), "a non-string method must be rejected");
}

#[test]
fn register_lsp_notification_hook_rejects_a_non_callable_proc() {
    let mut h = SteelCtxTestHarness::new();
    let msg = register_lsp_notification_hook(
        &mut h.ctx_init(),
        SteelVal::StringV("a/b".into()),
        SteelVal::IntV(1),
    )
    .expect_err("a non-callable proc must be rejected")
    .to_string();
    assert!(msg.contains("expected a callable"), "got: {msg}");
}

/// Registered from a plugin body, the entry belongs to that plugin, so
/// rollback of a failed activation removes it.
#[test]
fn register_lsp_notification_hook_attributes_the_running_plugin() {
    use crate::attribution::{EntryId, PluginId};
    let mut h = SteelCtxTestHarness::new();
    let id = PluginId::parse("core:myplugin").unwrap();
    h.plugin_stack.push(EntryId::main(id.clone()));
    register_lsp_notification_hook(
        &mut h.ctx(),
        SteelVal::StringV("a/b".into()),
        SteelVal::FuncV(noop_proc),
    )
    .expect("registration during plugin load must succeed");
    assert_eq!(
        h.registries.hooks.handlers_for("on-lsp-notification")[0].owner,
        Some(EntryId::main(id))
    );
}

/// The token builtins are `cmd`-gated through the real registration: called
/// from init.scm, each raises naming itself.
#[test]
fn the_tracking_token_builtins_reject_init_context() {
    for name in [
        "tracked-position-params",
        "untrack-position!",
        "keep-tracked-position!",
    ] {
        let mut host = crate::ScriptingHost::new();
        let mut null_host = crate::null_host::NullHost;
        let err = host
            .eval_source(&format!("({name} #f)"), &mut null_host)
            .expect_err("a cmd builtin must be rejected during init.scm eval");
        assert!(
            err.contains(&format!("{name}: not available during init")),
            "{name}: got: {err}"
        );
    }
}

#[test]
fn a_token_argument_that_is_not_an_integer_raises() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let err = untrack_position(&mut ctx, "not-a-token".into_steelval().unwrap())
        .expect_err("a string is no token");
    assert!(
        err.to_string().contains("untrack-position! token"),
        "got: {err}"
    );
}
