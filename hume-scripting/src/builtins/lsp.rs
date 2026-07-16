//! LSP server lifecycle (register/unregister/stop/restart/status), the
//! generic request/notify bridge, and read-only introspection. Decorations,
//! completion, edit/navigation primitives, and the minibuffer prompt live in
//! their own modules — LSP is a client of those, not their owner.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::json::json_to_steel;
use crate::types::{Effect, PendingLspNotify, PendingLspRequest, PendingLspServerOp};
use crate::{PendingLspServerReg, SteelCtx};

use super::args::{
    BidArg, json_params, list_to_strings, optional_json_arg, optional_string_arg, string_arg,
};

type SteelResult = Result<SteelVal, SteelErr>;

/// `(%register-lsp-server! language command args root-markers init-options settings)`
///
/// Callable from init.scm, plugin activation, or a command/hook body —
/// unlike `%define-language!`, this is not gated to init/activation-only.
/// Queues a last-wins registration: applied at the end of the *current*
/// eval (see `Editor::apply_lsp_server_op`), replacing any existing
/// registration for `language` and attaching already-open matching buffers.
/// `lsp-registered-for-language?` reads through the effect log, so it
/// reports this registration as live immediately, within the same eval.
///
/// All list args must be lists of strings. Pushes an
/// `Effect::LspServerOp(PendingLspServerOp::Register)`.
pub(crate) fn register_lsp_server(
    ctx: &mut SteelCtx,
    language: SteelVal,
    command: SteelVal,
    args_val: SteelVal,
    root_markers_val: SteelVal,
    init_options: SteelVal,
    settings: SteelVal,
) -> SteelResult {
    let language = string_arg(language, "register-lsp-server! language")?;
    let command = string_arg(command, "register-lsp-server! command")?;
    let args = list_to_strings(args_val, "register-lsp-server! args")?;
    let root_markers = list_to_strings(root_markers_val, "register-lsp-server! root-markers")?;
    let init_options = optional_json_arg(init_options, "register-lsp-server! init-options")?;
    let settings = optional_json_arg(settings, "register-lsp-server! settings")?;

    ctx.effects
        .push(Effect::LspServerOp(PendingLspServerOp::Register(
            PendingLspServerReg {
                language,
                command,
                args,
                root_markers,
                init_options,
                settings,
            },
        )));
    Ok(SteelVal::Void)
}

/// `(unregister-lsp-server! language)` — queues removal of `language`'s
/// registration and shutdown of any running clients for it, applied at the
/// end of the current eval (see `Editor::apply_lsp_server_op`).
///
/// Idempotent: unregistering a language with no registration and/or no
/// running clients is not an error — `:lsp-uninstall` of an already-removed
/// or never-installed server must succeed silently.
///
/// Applies at end-of-eval, so within the *same* eval the server process is
/// still alive. A caller that must touch the on-disk server files after
/// shutdown (e.g. reinstalling on Windows, where a running binary's file is
/// locked) should do that work in a follow-up queued eval (e.g. via
/// `(after 0 …)`), which runs strictly after this eval's drain reaps the
/// process.
pub(crate) fn unregister_lsp_server(ctx: &mut SteelCtx, language: SteelVal) -> SteelResult {
    let language = string_arg(language, "unregister-lsp-server! language")?;
    ctx.effects
        .push(Effect::LspServerOp(PendingLspServerOp::Unregister {
            language,
        }));
    Ok(SteelVal::Void)
}

/// `(lsp-stop! language)` — `language` a string, or `#f` for "the focused
/// buffer's attached server". Queues a stop, applied at the end of the
/// current eval (see `Editor::apply_lsp_server_op`); the report of how many
/// servers stopped is emitted by that same drain.
pub(crate) fn lsp_stop(ctx: &mut SteelCtx, language: SteelVal) -> SteelResult {
    let language = optional_string_arg(language, "lsp-stop! language")?;
    ctx.effects
        .push(Effect::LspServerOp(PendingLspServerOp::Stop { language }));
    Ok(SteelVal::Void)
}

/// `(lsp-restart! language)` — same argument shape as `lsp-stop!`. Queues a
/// stop-then-respawn, applied at the end of the current eval.
pub(crate) fn lsp_restart(ctx: &mut SteelCtx, language: SteelVal) -> SteelResult {
    let language = optional_string_arg(language, "lsp-restart! language")?;
    ctx.effects
        .push(Effect::LspServerOp(PendingLspServerOp::Restart {
            language,
        }));
    Ok(SteelVal::Void)
}

/// `(lsp-show-status!)` — queues opening the `[lsp-status]` read-only view,
/// applied at the end of the current eval.
pub(crate) fn lsp_show_status(ctx: &mut SteelCtx) -> SteelResult {
    ctx.effects
        .push(Effect::LspServerOp(PendingLspServerOp::ShowStatus));
    Ok(SteelVal::Void)
}

/// `(%lsp-request server method params callback allow-stale)`. The
/// `lsp-request` Scheme wrapper (BOOTSTRAP) supplies `#:allow-stale`'s
/// default. Pushes an `Effect::LspRequest`, sent by `Editor::send_one_lsp_request`
/// right after this eval returns — `SteelCtx` has no route to the transport
/// (crate fence), and queuing keeps every LSP send on one chokepoint
/// regardless of which eval kind (command, hook, or a queued callback)
/// triggered it.
pub(crate) fn lsp_request(
    ctx: &mut SteelCtx,
    server: SteelVal,
    method: SteelVal,
    params: SteelVal,
    callback: SteelVal,
    allow_stale: SteelVal,
    supersede: SteelVal,
) -> SteelResult {
    let server = optional_string_arg(server, "lsp-request server")?;
    let method = string_arg(method, "lsp-request method")?;
    let params = json_params(params, "lsp-request params")?;
    let allow_stale = match allow_stale {
        SteelVal::BoolV(b) => b,
        _ => steel::stop!(TypeMismatch => "lsp-request: #:allow-stale expected a bool"),
    };
    let supersede = optional_string_arg(supersede, "lsp-request supersede")?;
    ctx.effects.push(Effect::LspRequest(PendingLspRequest {
        server,
        method,
        params,
        callback,
        allow_stale,
        supersede,
    }));
    Ok(SteelVal::Void)
}

/// `(lsp-notify server method params)` — fire-and-forget, no callback, no
/// staleness tag (nothing to correlate a response against). Same queue
/// discipline as `lsp-request`.
pub(crate) fn lsp_notify(
    ctx: &mut SteelCtx,
    server: SteelVal,
    method: SteelVal,
    params: SteelVal,
) -> SteelResult {
    let server = optional_string_arg(server, "lsp-notify server")?;
    let method = string_arg(method, "lsp-notify method")?;
    let params = json_params(params, "lsp-notify params")?;
    ctx.effects.push(Effect::LspNotify(PendingLspNotify {
        server,
        method,
        params,
    }));
    Ok(SteelVal::Void)
}

/// `(on-lsp-notification method handler)` — registers `handler` for every
/// server notification of `method` that Rust doesn't already special-case
/// (window/logMessage, window/showMessage, $/progress, publishDiagnostics).
/// Persistent, immediate registration straight onto `ctx.registries` — same
/// init/plugin-load gate as `register-hook!`, no per-eval queue needed since
/// registration doesn't touch the transport.
pub(crate) fn on_lsp_notification(
    ctx: &mut SteelCtx,
    method: SteelVal,
    handler: SteelVal,
) -> SteelResult {
    let method = string_arg(method, "on-lsp-notification method")?;
    ctx.registries
        .lsp_notification_handlers
        .entry(method)
        .or_default()
        .push(handler);
    Ok(SteelVal::Void)
}

/// `(lsp-capabilities server)` → decoded `ServerCapabilities` hashmap, or
/// `#f` if `server` doesn't resolve or hasn't finished its handshake.
pub(crate) fn lsp_capabilities(ctx: &mut SteelCtx, server: SteelVal) -> SteelResult {
    let server = optional_string_arg(server, "lsp-capabilities server")?;
    Ok(
        match ctx
            .host
            .lsp()
            .and_then(|lsp| lsp.lsp_capabilities(server.as_deref()))
        {
            Some(json) => json_to_steel(&json),
            None => SteelVal::BoolV(false),
        },
    )
}

/// `(lsp-server-status)` → list of `{"language" "root" "state" "pending"}`.
pub(crate) fn lsp_server_status(ctx: &mut SteelCtx) -> SteelResult {
    let entries: Vec<SteelVal> = ctx
        .host
        .lsp()
        .map(|lsp| lsp.lsp_server_status())
        .unwrap_or_default()
        .into_iter()
        .map(|e| {
            let mut map = steel::HashMap::new();
            map.insert(
                SteelVal::StringV("language".into()),
                SteelVal::StringV(e.language.into()),
            );
            map.insert(
                SteelVal::StringV("root".into()),
                SteelVal::StringV(e.root.to_string_lossy().into_owned().into()),
            );
            map.insert(
                SteelVal::StringV("state".into()),
                SteelVal::StringV(e.state.into()),
            );
            map.insert(
                SteelVal::StringV("pending".into()),
                SteelVal::IntV(e.pending as isize),
            );
            SteelVal::HashMapV(steel::gc::Gc::new(map).into())
        })
        .collect();
    Ok(SteelVal::ListV(entries.into()))
}

/// `(lsp-server-for-buffer bid)` → registered language name, or `#f`.
pub(crate) fn lsp_server_for_buffer(ctx: &mut SteelCtx, bid: BidArg) -> SteelResult {
    let id = bid.0;
    Ok(
        match ctx.host.lsp().and_then(|lsp| lsp.lsp_server_for_buffer(id)) {
            Some(lang) => SteelVal::StringV(lang.into()),
            None => SteelVal::BoolV(false),
        },
    )
}

/// `(lsp-registered-for-language? language)` → bool. Registry query for the
/// `on-language-set` missing-server hint: distinguishes "no server
/// registered for this language" from "registered but still starting"
/// (`lsp-server-for-buffer` reports *attachment*, which can't make that
/// distinction). Reads through the `Effect::LspServerOp` entries queued this
/// eval/init, not yet applied, in emission order before falling back to the
/// live registry — the last queued `Register`/`Unregister` for `language`
/// wins, matching `Editor::apply_lsp_server_op`'s own last-wins semantics
/// exactly, so a same-eval registration is visible immediately instead of
/// only after the next drain.
///
/// Unlike its buffer/pane-touching siblings, this is a pure registry read
/// (no `EditorHost` state beyond the LSP registry itself), so its table
/// entry is `open` kind — no gate, callable during init/plugin load too.
/// That lets `core:lsp`'s own load-time scan (`registration.scm`) query it
/// directly to skip already-registered languages.
pub(crate) fn lsp_registered_for_language(ctx: &mut SteelCtx, language: SteelVal) -> SteelResult {
    let language = string_arg(language, "lsp-registered-for-language? language")?;
    let mut pending: Option<bool> = None;
    for effect in ctx.effects.iter() {
        let Effect::LspServerOp(op) = effect else {
            continue;
        };
        match op {
            PendingLspServerOp::Register(reg) if reg.language == language => pending = Some(true),
            PendingLspServerOp::Unregister { language: l } if *l == language => {
                pending = Some(false)
            }
            _ => {}
        }
    }
    let registered = match pending {
        Some(v) => v,
        None => ctx
            .host
            .lsp()
            .is_some_and(|lsp| lsp.lsp_registered_for_language(&language)),
    };
    Ok(SteelVal::BoolV(registered))
}

/// `(lsp-position-params bid)` → `{"textDocument" {"uri"} "position" {"line"
/// "character"}}` from `bid`'s primary cursor head, or `#f` if unavailable
/// (no attached server, no path, or not shown in any pane).
pub(crate) fn lsp_position_params(ctx: &mut SteelCtx, bid: BidArg) -> SteelResult {
    let id = bid.0;
    Ok(
        match ctx.host.lsp().and_then(|lsp| lsp.lsp_position_params(id)) {
            Some(json) => json_to_steel(&json),
            None => SteelVal::BoolV(false),
        },
    )
}

/// `(lsp-range-params bid)` → same shape but a `"range"` from the primary
/// selection.
pub(crate) fn lsp_range_params(ctx: &mut SteelCtx, bid: BidArg) -> SteelResult {
    let id = bid.0;
    Ok(
        match ctx.host.lsp().and_then(|lsp| lsp.lsp_range_params(id)) {
            Some(json) => json_to_steel(&json),
            None => SteelVal::BoolV(false),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::SteelCtxTestHarness;
    use steel::rvals::IntoSteelVal;

    fn list_of(items: &[&str]) -> SteelVal {
        items
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .into_steelval()
            .unwrap()
    }

    /// `Effect::LspServerOp` entries queued so far, in emission order.
    fn lsp_server_ops(h: &SteelCtxTestHarness) -> Vec<&PendingLspServerOp> {
        h.effects
            .iter()
            .filter_map(|e| match e {
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
            .filter_map(|e| match e {
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
        );
        assert!(result.is_ok());
        drop(ctx);
        let reg = expect_register(&h);
        assert_eq!(reg.language, "rust");
        assert_eq!(reg.command, "rust-analyzer");
        assert_eq!(reg.root_markers, vec!["Cargo.toml".to_string()]);
        assert_eq!(reg.init_options, None);
        assert_eq!(reg.settings, None);
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
        );
        assert!(result.is_ok());
        drop(ctx);
        let reg = expect_register(&h);
        assert_eq!(reg.init_options, Some(serde_json::json!({"a": 1})));
        assert_eq!(reg.settings, Some(serde_json::json!({"b": 2})));
    }

    /// `register-lsp-server!` is no longer init/activation-only — it must
    /// queue successfully from a plain command-mode context too, so that
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
            .push(crate::PluginId::Core("lsp".to_string()));
        let result = register_lsp_server(
            &mut ctx,
            "rust".into_steelval().unwrap(),
            "rust-analyzer".into_steelval().unwrap(),
            list_of(&[]),
            list_of(&[]),
            SteelVal::BoolV(false),
            SteelVal::BoolV(false),
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
        ctx.effects.push(pending_register("rust"));
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
        ctx.effects.push(pending_register("rust"));
        ctx.effects.push(pending_unregister("rust"));
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
        ctx.effects.push(pending_unregister("rust"));
        ctx.effects.push(pending_register("rust"));
        let result = lsp_registered_for_language(&mut ctx, "rust".into_steelval().unwrap());
        assert_eq!(result.unwrap(), SteelVal::BoolV(true));
    }

    /// A queued op for a *different* language must not affect the answer.
    #[test]
    fn a_queued_op_for_a_different_language_does_not_flip_the_answer() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        ctx.effects.push(pending_register("python"));
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
        ctx.effects
            .push(Effect::LspServerOp(PendingLspServerOp::Stop {
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
}
