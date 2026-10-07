//! LSP server lifecycle (register/unregister/stop/restart/status), the
//! generic request/notify bridge, and read-only introspection. Decorations,
//! completion, edit/navigation primitives, and the minibuffer prompt live in
//! their own modules. LSP is a client of those, not their owner.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::host::{PositionParams, RangeParams, RangesParams};
use crate::json::steel_to_params;
use crate::types::{
    CapabilityQuery, Effect, FeatureFilter, ListEntry, ListLayer, LspFeature, LspFeatureSet,
    LspServerTarget, PaneHandle, Params, PendingLspNotify, PendingLspRequest, PendingLspServerOp,
    RequestMode, RequestParams, RouteSpec, ServerName, WhenUnavailable,
};
use crate::{PendingLspServerReg, SteelCtx};

use super::SteelResult;
use super::args::{
    bool_arg, callable_arg, hash_entry, json_arg, list_items, list_of, list_to_env_pairs,
    list_to_strings, optional_json_arg, optional_list_items, optional_server_arg,
    optional_string_arg, optional_symbol_arg, optional_token_arg, pair_fields, server_arg,
    string_arg, string_hash, string_list, symbol_enum_arg, symbol_hash, token_arg, wire_position,
};
use super::errors::generic_err;
use super::hooks::{register_entry, require_known_event};
use super::ids::{DocRange, ServerRef, SteelPane};

/// A registration or list-entry server name, as [`ServerName::parse`]
/// accepts it.
fn server_name_arg(val: SteelVal, ctx_name: &str) -> Result<ServerName, SteelErr> {
    let name = string_arg(val, ctx_name)?;
    ServerName::parse(&name).map_err(|e| generic_err(format!("{ctx_name}: {e}")))
}

/// `(%register-lsp-server! name command args init-options settings env)`
///
/// Callable from init.scm, plugin activation, or a command/hook body,
/// unlike `%define-language!`, this is not gated to init/activation-only.
/// Queues a last-wins registration keyed by `name`: applied at the end of
/// the *current* eval (see `Editor::apply_lsp_server_ops`), replacing any
/// existing registration of `name` and attaching already-open matching
/// buffers. `lsp-server-registered?` reads through the effect log, so it
/// reports this registration as live immediately, within the same eval.
///
/// `args` is a list of strings; `env` is a list of
/// `("KEY" . "VALUE")` dotted pairs, applied additively to the spawned
/// process's inherited environment. Pushes an
/// `Effect::LspServerOp(PendingLspServerOp::Register)`.
// Each param is a positional/keyword arg the `builtins!` table maps 1:1 from
// `register-lsp-server!`'s own Steel signature. Bundling them into a struct
// would break that direct correspondence for no benefit, since every arg is
// already decoded and validated independently right below.
#[allow(clippy::too_many_arguments)]
pub(crate) fn register_lsp_server(
    ctx: &mut SteelCtx,
    name: SteelVal,
    command: SteelVal,
    args_val: SteelVal,
    init_options: SteelVal,
    settings: SteelVal,
    env_val: SteelVal,
) -> SteelResult {
    let name = server_name_arg(name, "register-lsp-server! name")?;
    let command = string_arg(command, "register-lsp-server! command")?;
    let args = list_to_strings(args_val, "register-lsp-server! args")?;
    let init_options = optional_json_arg(init_options, "register-lsp-server! init-options")?;
    let settings = optional_json_arg(settings, "register-lsp-server! settings")?;
    let env = list_to_env_pairs(env_val, "register-lsp-server! env")?;

    ctx.push_effect(Effect::LspServerOp(PendingLspServerOp::Register(
        PendingLspServerReg {
            name,
            command,
            args,
            init_options,
            settings,
            env,
        },
    )));
    Ok(SteelVal::Void)
}

/// `(unregister-lsp-server! name)`: queues removal of `name`'s
/// registration, applied at the end of the current eval (see
/// `Editor::apply_lsp_server_ops`). Every buffer attached to an instance of
/// `name` detaches, and an instance no buffer uses stops. The stop happens
/// where the op falls among the eval's other server ops, so a
/// `register-lsp-server!` of the same name after it, in the same eval, spawns
/// a fresh server.
///
/// Idempotent: unregistering a name with no registration and/or no
/// running instance is not an error: `:lsp-uninstall` of an already-removed
/// or never-installed server must succeed silently.
///
/// Applies at end-of-eval, so within the *same* eval the server process is
/// still alive. A caller that must touch the on-disk server files after
/// shutdown (e.g. reinstalling on Windows, where a running binary's file is
/// locked) should do that work in a follow-up queued eval (e.g. via
/// `(after! 0 …)`), which runs strictly after this eval's drain reaps the
/// process.
pub(crate) fn unregister_lsp_server(ctx: &mut SteelCtx, name: SteelVal) -> SteelResult {
    let name = server_name_arg(name, "unregister-lsp-server! name")?;
    ctx.push_effect(Effect::LspServerOp(PendingLspServerOp::Unregister { name }));
    Ok(SteelVal::Void)
}

/// One server-list entry: a server name, or a hash with a `'name` and at
/// most one of `'only-features`/`'except-features`. Every shape error
/// raises, so a typo cannot widen a filter.
fn list_entry_arg(item: SteelVal, ctx_name: &str) -> Result<ListEntry, SteelErr> {
    if !matches!(item, SteelVal::HashMapV(_)) {
        return Ok(ListEntry {
            name: server_name_arg(item, ctx_name)?,
            filter: FeatureFilter::All,
        });
    }
    let entry = hash_entry(
        item,
        ctx_name,
        &["name", "only-features", "except-features"],
    )?;
    let name = server_name_arg(entry.required("name")?, ctx_name)?;
    let filter = match (
        entry.optional("only-features"),
        entry.optional("except-features"),
    ) {
        (None, None) => FeatureFilter::All,
        (Some(only), None) => {
            FeatureFilter::Only(feature_set_arg(only, "only-features", ctx_name)?)
        }
        (None, Some(except)) => {
            FeatureFilter::Except(feature_set_arg(except, "except-features", ctx_name)?)
        }
        (Some(_), Some(_)) => {
            return Err(generic_err(format!(
                "{ctx_name}: '{name}' gives both 'only-features and 'except-features"
            )));
        }
    };
    Ok(ListEntry { name, filter })
}

/// A list of feature names, the value of a list entry's `key`.
fn feature_set_arg(val: SteelVal, key: &str, ctx_name: &str) -> Result<LspFeatureSet, SteelErr> {
    let label = format!("{ctx_name} '{key}");
    list_to_strings(val, &label)?
        .iter()
        .map(|name| symbol_enum_arg(name, &label, &LspFeature::NAMED))
        .collect()
}

/// The one implementation behind `set-language-servers!` and
/// `set-default-language-servers!`: they differ only in `layer`. `entries`
/// is a list of server names/hashes, or `#f` to clear the layer.
fn queue_language_servers(
    ctx: &mut SteelCtx,
    language: SteelVal,
    entries: SteelVal,
    layer: ListLayer,
    ctx_name: &str,
) -> SteelResult {
    let language = string_arg(language, ctx_name)?;
    let entries = optional_list_items(entries, ctx_name)?
        .map(|items| {
            let mut decoded: Vec<ListEntry> = Vec::with_capacity(items.len());
            for item in items {
                let entry = list_entry_arg(item, ctx_name)?;
                if decoded.iter().any(|e| e.name == entry.name) {
                    return Err(generic_err(format!(
                        "{ctx_name}: '{}' is listed twice",
                        entry.name
                    )));
                }
                decoded.push(entry);
            }
            Ok(decoded)
        })
        .transpose()?;
    ctx.push_effect(Effect::LspServerOp(
        PendingLspServerOp::SetLanguageServers {
            language,
            layer,
            entries,
        },
    ));
    Ok(SteelVal::Void)
}

/// `(set-language-servers! language entries)`: the user's own ordered server
/// list for `language`, which beats any default list whatever the call order.
pub(crate) fn set_language_servers(
    ctx: &mut SteelCtx,
    language: SteelVal,
    entries: SteelVal,
) -> SteelResult {
    queue_language_servers(
        ctx,
        language,
        entries,
        ListLayer::User,
        "set-language-servers!",
    )
}

/// `(set-default-language-servers! language entries)`: the list a plugin
/// ships for `language`, used only while the user has set none.
pub(crate) fn set_default_language_servers(
    ctx: &mut SteelCtx,
    language: SteelVal,
    entries: SteelVal,
) -> SteelResult {
    queue_language_servers(
        ctx,
        language,
        entries,
        ListLayer::Default,
        "set-default-language-servers!",
    )
}

/// `(lsp-language-servers language)` → the servers `language`'s buffers
/// attach to, in order: one `(hash 'name n)` per server, with
/// `'only-features` or `'except-features` (a list of feature symbols) when
/// its list entry filters it.
pub(crate) fn lsp_language_servers(ctx: &mut SteelCtx, language: SteelVal) -> SteelResult {
    let language = string_arg(language, "lsp-language-servers language")?;
    let entries = ctx
        .host
        .lsp()
        .map(|lsp| lsp.lsp_language_servers(&language))
        .unwrap_or_default();
    let features =
        |set: LspFeatureSet| list_of(set.iter().map(|f| SteelVal::SymbolV(f.name().into())));
    Ok(list_of(entries.into_iter().map(|entry| {
        let name = ("name", SteelVal::StringV(entry.name.as_str().into()));
        match entry.filter {
            FeatureFilter::All => symbol_hash([name]),
            FeatureFilter::Only(set) => symbol_hash([name, ("only-features", features(set))]),
            FeatureFilter::Except(set) => symbol_hash([name, ("except-features", features(set))]),
        }
    })))
}

/// `(lsp-stop! target)`: `target` a pane (every server attached to its
/// buffer) or a string/symbol (every running instance of that server name).
/// Queues a stop, applied at the end of the current eval (see
/// `Editor::apply_lsp_server_ops`); the report of how many servers stopped is
/// emitted by that same drain.
pub(crate) fn lsp_stop(ctx: &mut SteelCtx, target: LspServerTarget) -> SteelResult {
    ctx.push_effect(Effect::LspServerOp(PendingLspServerOp::Stop { target }));
    Ok(SteelVal::Void)
}

/// `(lsp-restart! target)`: same argument shape as `lsp-stop!`. Queues a
/// stop-then-respawn, applied at the end of the current eval.
pub(crate) fn lsp_restart(ctx: &mut SteelCtx, target: LspServerTarget) -> SteelResult {
    ctx.push_effect(Effect::LspServerOp(PendingLspServerOp::Restart { target }));
    Ok(SteelVal::Void)
}

/// `(lsp-show-status! pane)`: queues opening the `[lsp-status]` read-only
/// view, applied at the end of the current eval. Kind-A: `pane` must be the
/// focused pane, since the view opens anchored there.
pub(crate) fn lsp_show_status(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    ctx.host
        .buffers()
        .require_focused_pane(pane)
        .map_err(generic_err)?;
    ctx.push_effect(Effect::LspServerOp(PendingLspServerOp::ShowStatus));
    Ok(SteelVal::Void)
}

/// `#:feature`: a feature symbol, or `#f` for none.
fn feature_arg(val: SteelVal, ctx_name: &str) -> Result<Option<LspFeature>, SteelErr> {
    optional_symbol_arg(val, ctx_name)?
        .map(|name| symbol_enum_arg(&name, ctx_name, &LspFeature::NAMED))
        .transpose()
}

/// A feature symbol that must be given.
pub(super) fn required_feature_arg(val: SteelVal, ctx_name: &str) -> Result<LspFeature, SteelErr> {
    feature_arg(val, ctx_name)?
        .ok_or_else(|| generic_err(format!("{ctx_name}: expected a feature symbol")))
}

/// The routing keywords every request and notification shares. `to` is
/// `#f` for `lsp-request-all!`, which takes no `#:to`.
fn route_arg(feature: SteelVal, to: SteelVal, verb: &str) -> Result<RouteSpec, SteelErr> {
    Ok(RouteSpec {
        feature: feature_arg(feature, &format!("{verb} #:feature"))?,
        to: optional_server_arg(to, &format!("{verb} #:to"))?,
    })
}

/// A `#:feature` names the feature of a method the editor has none for. A
/// standard method already has one, and a second could only disagree.
fn reject_feature_on_standard_method(
    ctx: &mut SteelCtx,
    route: &RouteSpec,
    method: &str,
    verb: &str,
) -> Result<(), SteelErr> {
    if route.feature.is_none() {
        return Ok(());
    }
    match ctx
        .host
        .lsp()
        .and_then(|lsp| lsp.lsp_method_feature(method))
    {
        Some(feature) => Err(generic_err(format!(
            "{verb}: {method} is a standard method of the '{} feature; #:feature is for other methods",
            feature.name()
        ))),
        None => Ok(()),
    }
}

/// Converts `val` to request params, so a malformed value raises at the
/// call rather than when the request is sent.
fn checked_params(val: SteelVal, ctx_name: &str) -> Result<Params, SteelErr> {
    if matches!(val, SteelVal::BoolV(_)) {
        steel::stop!(TypeMismatch => "{}: expected a hashmap or JSON handle, got a boolean", ctx_name);
    }
    steel_to_params(&val).map_err(|e| generic_err(format!("{ctx_name}: {e}")))
}

/// The one decode behind `%lsp-request!` and `%lsp-request-all!`: the
/// options both share, queued with the mode and params each decoded.
// Each param is a keyword the bootstrap wrappers pass through positionally.
#[allow(clippy::too_many_arguments)]
fn queue_request(
    ctx: &mut SteelCtx,
    pane: PaneHandle,
    method: SteelVal,
    params: RequestParams,
    mode: RequestMode,
    route: RouteSpec,
    callback: SteelVal,
    unavailable: SteelVal,
    allow_stale: SteelVal,
    supersede: SteelVal,
    require_focus: SteelVal,
    tracked: SteelVal,
) -> SteelResult {
    let verb = mode.verb();
    let method = string_arg(method, &format!("{verb} method"))?;
    reject_feature_on_standard_method(ctx, &route, &method, verb)?;
    let callback = callable_arg(callback, &format!("{verb} callback"))?;
    let ctx_name = format!("{verb} #:unavailable");
    let when_unavailable = optional_symbol_arg(unavailable, &ctx_name)?
        .map(|name| symbol_enum_arg(&name, &ctx_name, &WhenUnavailable::NAMED))
        .transpose()?
        .unwrap_or_default();
    let allow_stale = bool_arg(allow_stale, &format!("{verb} #:allow-stale"))?;
    let supersede = optional_string_arg(supersede, &format!("{verb} #:supersede"))?;
    let require_focus = bool_arg(require_focus, &format!("{verb} #:require-focus"))?
        .then(|| {
            pane.pane().ok_or_else(|| {
                generic_err(format!(
                    "{verb}: #:require-focus needs a pane, but was given none"
                ))
            })
        })
        .transpose()?;
    let tracked = optional_token_arg(tracked, &format!("{verb} #:tracked"))?;
    ctx.push_effect(Effect::LspRequest(PendingLspRequest {
        bid: pane.buffer(),
        method,
        params,
        mode,
        route,
        when_unavailable,
        callback,
        allow_stale,
        supersede,
        require_focus,
        tracked,
    }));
    Ok(SteelVal::Void)
}

/// `(%lsp-request! pane method params callback feature to unavailable
/// allow-stale supersede require-focus tracked)`, behind the `lsp-request!`
/// wrapper (BOOTSTRAP), which supplies the keyword defaults. Queues one
/// request, sent after this eval to the first server attached to `pane`'s
/// buffer that `method`, `#:feature` and `#:to` admit and that is running
/// then. `callback` receives `(err result)`. `#:unavailable 'empty` answers
/// a request no server can take with void and no error, instead of an
/// `'unavailable` error.
///
/// `#:require-focus` fires the callback only if `pane` is still focused when
/// the response arrives: for cursor-anchored UI (hover, signature help, code
/// actions), not background requests. It raises at once if `pane` carries no
/// pane.
///
/// `#:tracked` names a `track-position!` token the request holds: the editor
/// releases it once the callback has run, raised, or will never run, unless
/// the callback kept it with `keep-tracked-position!`.
// Each param is a positional/keyword arg the `builtins!` table maps 1:1 from
// `lsp-request!`'s own Steel signature, same rationale as
// `register_lsp_server`'s own `#[allow]`, just above in this file.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lsp_request(
    ctx: &mut SteelCtx,
    pane: PaneHandle,
    method: SteelVal,
    params: SteelVal,
    callback: SteelVal,
    feature: SteelVal,
    to: SteelVal,
    unavailable: SteelVal,
    allow_stale: SteelVal,
    supersede: SteelVal,
    require_focus: SteelVal,
    tracked: SteelVal,
) -> SteelResult {
    let route = route_arg(feature, to, RequestMode::Single.verb())?;
    let params = RequestParams::Shared(checked_params(params, "lsp-request! params")?);
    queue_request(
        ctx,
        pane,
        method,
        params,
        RequestMode::Single,
        route,
        callback,
        unavailable,
        allow_stale,
        supersede,
        require_focus,
        tracked,
    )
}

/// `(%lsp-request-all! pane method params callback feature unavailable
/// allow-stale supersede require-focus tracked)`, behind the
/// `lsp-request-all!` wrapper. Sends to every server `method` and
/// `#:feature` admit and calls `callback` once with `(err results)`:
/// one `(hash 'server s 'err e 'result r)` per server, in attachment order.
/// `params` is a hash every server receives, or a list of
/// `(server . params)` pairs giving each named server its own; `method` and
/// `#:feature` still admit or reject each of them. `#:unavailable 'empty`
/// calls back with `'()` and no error when no server can take it.
// Same rationale as `lsp_request`'s `#[allow]`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lsp_request_all(
    ctx: &mut SteelCtx,
    pane: PaneHandle,
    method: SteelVal,
    params: SteelVal,
    callback: SteelVal,
    feature: SteelVal,
    unavailable: SteelVal,
    allow_stale: SteelVal,
    supersede: SteelVal,
    require_focus: SteelVal,
    tracked: SteelVal,
) -> SteelResult {
    const CTX: &str = "lsp-request-all! params";
    let route = route_arg(feature, SteelVal::BoolV(false), RequestMode::All.verb())?;
    let params = match params {
        SteelVal::ListV(items) => {
            let mut pairs: Vec<(crate::ServerRef, Params)> = Vec::new();
            for item in items {
                let (server, params) = pair_fields(item, CTX, "(server . params)")?;
                let server = server_arg(&server, CTX).map_err(|_| {
                    generic_err(format!("{CTX}: each entry must be (server . params)"))
                })?;
                if pairs.iter().any(|(listed, _)| listed.id == server.id) {
                    return Err(generic_err(format!(
                        "{CTX}: '{}' is listed twice",
                        server.name
                    )));
                }
                pairs.push((server, checked_params(params, CTX)?));
            }
            if pairs.is_empty() {
                return Err(generic_err(format!(
                    "{CTX}: the (server . params) list is empty"
                )));
            }
            RequestParams::PerServer(pairs)
        }
        other => RequestParams::Shared(checked_params(other, CTX)?),
    };
    queue_request(
        ctx,
        pane,
        method,
        params,
        RequestMode::All,
        route,
        callback,
        unavailable,
        allow_stale,
        supersede,
        require_focus,
        tracked,
    )
}

/// `(%lsp-notify! pane method params feature to)`, behind the `lsp-notify!`
/// wrapper: fire-and-forget to every server `#:feature` and `#:to` admit,
/// with no callback and no staleness tag (nothing to correlate a response
/// against).
pub(crate) fn lsp_notify(
    ctx: &mut SteelCtx,
    pane: PaneHandle,
    method: SteelVal,
    params: SteelVal,
    feature: SteelVal,
    to: SteelVal,
) -> SteelResult {
    const VERB: &str = "lsp-notify!";
    let method = string_arg(method, "lsp-notify! method")?;
    let params = checked_params(params, "lsp-notify! params")?;
    let route = route_arg(feature, to, VERB)?;
    reject_feature_on_standard_method(ctx, &route, &method, VERB)?;
    ctx.push_effect(Effect::LspNotify(PendingLspNotify {
        bid: pane.buffer(),
        method,
        params,
        route,
    }));
    Ok(SteelVal::Void)
}

/// `(%lsp-servers pane feature method)`, behind the `lsp-servers` wrapper:
/// the servers attached to `pane`'s buffer, in order. With neither keyword,
/// every attached server whatever its state; with either, the servers a
/// request of that feature or method would reach now.
pub(crate) fn lsp_servers(
    ctx: &mut SteelCtx,
    pane: PaneHandle,
    feature: SteelVal,
    method: SteelVal,
) -> SteelResult {
    let feature = feature_arg(feature, "lsp-servers #:feature")?;
    let method = optional_string_arg(method, "lsp-servers #:method")?;
    let servers = match ctx.host.lsp() {
        Some(lsp) => lsp
            .lsp_servers(pane.buffer(), feature, method.as_deref())
            .map_err(generic_err)?,
        None => Vec::new(),
    };
    Ok(list_of(servers.into_iter().map(ServerRef::into_steel_val)))
}

/// `(lsp-server-name server)` → the name `server` was registered under.
pub(crate) fn lsp_server_name(server: SteelVal) -> SteelResult {
    let server = server_arg(&server, "lsp-server-name")?;
    Ok(SteelVal::StringV(server.name.as_str().into()))
}

/// The event `register-lsp-notification-hook!` registers on. This crate
/// compiles in no event list, so the name is checked against the host's on
/// every call and a rename in the editor fails loudly here.
const LSP_NOTIFICATION_EVENT: &str = "on-lsp-notification";

/// `(register-lsp-notification-hook! methods proc)`: an `on-lsp-notification`
/// handler that fires only for a notification whose method is `methods` (a
/// string) or one of `methods` (a list of strings). `proc` receives
/// `(server method params)` like a `register-hook!` handler, so one proc can
/// serve several methods. Same registry, init/plugin-load gate and plugin
/// ownership as `register-hook!`: a failed plugin's handler is removed with
/// it, filter included.
pub(crate) fn register_lsp_notification_hook(
    ctx: &mut SteelCtx,
    methods: SteelVal,
    proc: SteelVal,
) -> SteelResult {
    const VERB: &str = "register-lsp-notification-hook!";
    let methods = match methods {
        SteelVal::StringV(s) => vec![s.to_string()],
        other => list_to_strings(other, &format!("{VERB} methods"))?,
    };
    if methods.is_empty() {
        steel::stop!(Generic => "{VERB}: methods must not be empty");
    }
    let proc = callable_arg(proc, &format!("{VERB} proc"))?;
    require_known_event(ctx, LSP_NOTIFICATION_EVENT, VERB)?;
    register_entry(
        ctx,
        LSP_NOTIFICATION_EVENT,
        proc,
        Some(methods.into_boxed_slice()),
    );
    Ok(SteelVal::Void)
}

/// `(lsp-capabilities server)` → an opaque `JsonHandle` onto `server`'s
/// wire `ServerCapabilities`, or `#f` once it has stopped. Read it with
/// `json-ref`/`json-contains?`.
pub(crate) fn lsp_capabilities(ctx: &mut SteelCtx, server: SteelVal) -> SteelResult {
    let server = server_arg(&server, "lsp-capabilities")?;
    Ok(ctx
        .host
        .lsp()
        .and_then(|lsp| lsp.lsp_capabilities(server.id))
        .map_or(SteelVal::BoolV(false), |v| {
            crate::json::to_steel_handle(v, crate::json::WireOrigin::Local)
        }))
}

/// `(%lsp-capability server feature method)`, behind the `lsp-capability`
/// wrapper: what `server` advertises for `feature`, or for the one
/// capability `method` needs, as the provider's wire value (`#t`, or a
/// handle onto its options object); `#f` when it advertises none or has
/// stopped. Exactly one keyword is given.
pub(crate) fn lsp_capability(
    ctx: &mut SteelCtx,
    server: SteelVal,
    feature: SteelVal,
    method: SteelVal,
) -> SteelResult {
    let server = server_arg(&server, "lsp-capability")?;
    let feature = feature_arg(feature, "lsp-capability #:feature")?;
    let method = optional_string_arg(method, "lsp-capability #:method")?;
    let query = match (feature, method.as_deref()) {
        (Some(feature), None) => CapabilityQuery::Feature(feature),
        (None, Some(method)) => CapabilityQuery::Method(method),
        _ => {
            return Err(generic_err(
                "lsp-capability: give exactly one of #:feature and #:method".to_string(),
            ));
        }
    };
    Ok(ctx
        .host
        .lsp()
        .and_then(|lsp| lsp.lsp_capability(server.id, query))
        .map_or(SteelVal::BoolV(false), |v| {
            crate::json::to_steel_handle(v, crate::json::WireOrigin::Local)
        }))
}

/// `(lsp-server-status)` → list of `(hash 'name 'languages 'root 'state 'pending)`.
pub(crate) fn lsp_server_status(ctx: &mut SteelCtx) -> SteelResult {
    let entries: Vec<SteelVal> = ctx
        .host
        .lsp()
        .map(|lsp| lsp.lsp_server_status())
        .unwrap_or_default()
        .into_iter()
        .map(|e| {
            symbol_hash([
                ("name", SteelVal::StringV(e.name.as_str().into())),
                ("languages", string_list(e.languages)),
                (
                    "root",
                    SteelVal::StringV(e.root.to_string_lossy().into_owned().into()),
                ),
                ("state", SteelVal::SymbolV(e.state.into())),
                ("pending", SteelVal::IntV(e.pending as isize)),
            ])
        })
        .collect();
    Ok(SteelVal::ListV(entries.into()))
}

/// `(lsp-server-registered? name)` → bool. Reads through the
/// `Effect::LspServerOp` entries queued this eval, in emission order, before
/// falling back to the live registry: the last queued `Register`/
/// `Unregister` of `name` wins, matching `Editor::apply_lsp_server_ops`'s own
/// last-wins semantics, so a same-eval registration is visible at once.
///
/// A pure registry read, so its table entry is `open` kind: callable during
/// init/plugin load too, which lets `core:lsp-install`'s load-time scan skip
/// names already registered.
pub(crate) fn lsp_server_registered(ctx: &mut SteelCtx, name: SteelVal) -> SteelResult {
    let name = server_name_arg(name, "lsp-server-registered? name")?;
    let queued = ctx
        .effects
        .iter()
        .rev()
        .find_map(|queued| match &queued.effect {
            Effect::LspServerOp(PendingLspServerOp::Register(reg)) if reg.name == name => {
                Some(true)
            }
            Effect::LspServerOp(PendingLspServerOp::Unregister { name: n }) if *n == name => {
                Some(false)
            }
            _ => None,
        });
    let registered = match queued {
        Some(v) => v,
        None => ctx
            .host
            .lsp()
            .is_some_and(|lsp| lsp.lsp_server_registered(&name)),
    };
    Ok(SteelVal::BoolV(registered))
}

/// The JSON-shaped `{"uri" uri}` a params hash names its document by.
fn text_document(uri: String) -> SteelVal {
    string_hash([("uri", SteelVal::StringV(uri.into()))])
}

/// `{"textDocument" {"uri"} "position" <position>}`: a position stays an
/// opaque value until the request is sent.
pub(crate) fn position_params_value(params: PositionParams) -> SteelVal {
    string_hash([
        ("textDocument", text_document(params.uri)),
        ("position", params.pos.into_steel_val()),
    ])
}

fn range_params_value(params: RangeParams) -> SteelVal {
    string_hash([
        ("textDocument", text_document(params.uri)),
        ("range", params.range.into_steel_val()),
    ])
}

fn ranges_params_value(params: RangesParams) -> SteelVal {
    string_hash([
        ("textDocument", text_document(params.uri)),
        (
            "ranges",
            list_of(params.ranges.into_iter().map(DocRange::into_steel_val)),
        ),
    ])
}

/// A kind-B params read as a Steel value: `Err` (a stale `pane`) raises,
/// `Ok(None)` (a buffer with no path) and no LSP host are `#f`.
fn params_result<T>(
    result: Option<Result<Option<T>, String>>,
    to_steel: fn(T) -> SteelVal,
) -> SteelResult {
    match result {
        None => Ok(SteelVal::BoolV(false)),
        Some(Err(e)) => Err(generic_err(e)),
        Some(Ok(params)) => Ok(params.map_or(SteelVal::BoolV(false), to_steel)),
    }
}

/// `(lsp-position-params pane)` → `{"textDocument" {"uri"} "position" p}`
/// for the primary cursor head in `pane`'s own pane, `p` an opaque position
/// each server receives in its own encoding, or `#f` for a buffer with no
/// path. Raises (kind-B fail-fast) when `pane` carries no pane, a closed
/// one, or one that no longer shows its buffer.
pub(crate) fn lsp_position_params(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    params_result(
        ctx.host.lsp().map(|lsp| lsp.lsp_position_params(pane)),
        position_params_value,
    )
}

/// `(track-position! pane)` → a token naming the primary cursor head in
/// `pane`'s own pane, carried through every edit, or `#f` when the host
/// tracks nothing. Raises (kind-B fail-fast) when `pane` carries no pane, a
/// closed one, or one that no longer shows its buffer.
pub(crate) fn track_position(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    match ctx.host.lsp().map(|lsp| lsp.track_position(pane)) {
        None => Ok(SteelVal::BoolV(false)),
        Some(Err(e)) => Err(generic_err(e)),
        Some(Ok(token)) => Ok(token.to_steel()),
    }
}

/// `(tracked-position-params token)` → the `lsp-position-params` shape for
/// where the tracked position is now, or `#f` for a released or unknown
/// token, a closed or replaced buffer, or a buffer with no path. A `#f`
/// token is that same stale case; only a non-integer raises.
pub(crate) fn tracked_position_params(ctx: &mut SteelCtx, token: SteelVal) -> SteelResult {
    let token = token_arg(token, "tracked-position-params token")?;
    Ok(ctx
        .host
        .lsp()
        .and_then(|lsp| lsp.tracked_position_params(token))
        .map_or(SteelVal::BoolV(false), position_params_value))
}

/// `(keep-tracked-position! token)`: keeps a position an `lsp-request!`
/// holds through `#:tracked` past its callback; the caller releases it with
/// `untrack-position!`. A `#f`, released or unknown token is a no-op.
pub(crate) fn keep_tracked_position(ctx: &mut SteelCtx, token: SteelVal) -> SteelResult {
    let token = token_arg(token, "keep-tracked-position! token")?;
    if let Some(lsp) = ctx.host.lsp() {
        lsp.keep_tracked_position(token);
    }
    Ok(SteelVal::Void)
}

/// `(untrack-position! token)`: releases the position. A `#f`, released or
/// unknown token is a no-op.
pub(crate) fn untrack_position(ctx: &mut SteelCtx, token: SteelVal) -> SteelResult {
    let token = token_arg(token, "untrack-position! token")?;
    if let Some(lsp) = ctx.host.lsp() {
        lsp.untrack_position(token);
    }
    Ok(SteelVal::Void)
}

/// `(lsp-primary-range-params pane)` → same shape but a `"range"` from the
/// primary selection alone, an opaque range value.
pub(crate) fn lsp_primary_range_params(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    params_result(
        ctx.host.lsp().map(|lsp| lsp.lsp_primary_range_params(pane)),
        range_params_value,
    )
}

/// `(lsp-linewise-ranges-params pane)` → `{"textDocument" {"uri"} "ranges"
/// [...]}`, one opaque range per linewise selection in `pane`'s own pane
/// (touching selections coalesced into one range apiece). Carries no
/// all/none/mixed verdict. `:lsp-fmt` gets that from `selections-linewise?`/
/// `selections-charwise?` instead, since every `lsp-*-params` builtin's
/// return value is a wire-ready params hash forwarded to `lsp-request!`
/// unchanged or with a protocol key inserted, and a non-protocol key here
/// would break that (see `CursorHost::selections_linewise`'s doc comment).
pub(crate) fn lsp_linewise_ranges_params(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    params_result(
        ctx.host
            .lsp()
            .map(|lsp| lsp.lsp_linewise_ranges_params(pane)),
        ranges_params_value,
    )
}

/// A non-negative JSON integer, for a field read directly off a
/// `serde_json::Value` rather than decoded through a `SteelVal` arg (see
/// `super::args::usize_arg`'s Steel-side counterpart).
fn json_usize(v: &serde_json::Value, ctx_name: &str) -> Result<usize, SteelErr> {
    v.as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| {
            generic_err(format!(
                "{ctx_name}: expected a non-negative integer, got {v}"
            ))
        })
}

/// `(lsp-position->offset pane position)` → `pane`'s buffer's char offset
/// for the wire `{"line" "character"}` hashmap `position`, decoded in
/// `position`'s own tagged producing-server encoding (`Err` if `position`
/// isn't a value from an LSP server response; see
/// `JsonHandle::position_encoding`), or `#f` if `position` would land on
/// the buffer's trailing phantom line (a stale response racing an edit, or a
/// server's past-end convention). Every point-anchored decoration setter
/// (`set-inlay-hints!`) rejects that offset outright, so refusing here lets
/// a caller filter one bad entry instead of the whole setter call failing
/// on it.
pub(crate) fn lsp_position_to_offset(
    ctx: &mut SteelCtx,
    pane: PaneHandle,
    position: SteelVal,
) -> SteelResult {
    let bid = pane.buffer();
    let handle = json_arg(position, "lsp-position->offset")?;
    let encoding = handle
        .position_encoding("lsp-position->offset")
        .map_err(generic_err)?;
    let pos = wire_position(handle.value(), "lsp-position->offset")?;
    Ok(
        match ctx
            .host
            .lsp()
            .and_then(|lsp| lsp.lsp_wire_point_to_char(bid, pos, encoding))
        {
            Some(offset) => SteelVal::IntV(offset as isize),
            None => SteelVal::BoolV(false),
        },
    )
}

/// `(lsp-range->offsets pane range)` → `(hash 'start s 'end e)` half-open char
/// offsets for the wire `{"start" {"line" "character"} "end" {"line"
/// "character"}}` hashmap `range`, same encoding rule as
/// `lsp-position->offset`, decoded in `range`'s own tagged producing-server
/// encoding.
pub(crate) fn lsp_range_to_offsets(
    ctx: &mut SteelCtx,
    pane: PaneHandle,
    range: SteelVal,
) -> SteelResult {
    let bid = pane.buffer();
    let handle = json_arg(range, "lsp-range->offsets")?;
    let encoding = handle
        .position_encoding("lsp-range->offsets")
        .map_err(generic_err)?;
    let range_json = handle.value();
    let start_json = range_json
        .get("start")
        .ok_or_else(|| generic_err("lsp-range->offsets: range missing 'start'"))?;
    let end_json = range_json
        .get("end")
        .ok_or_else(|| generic_err("lsp-range->offsets: range missing 'end'"))?;
    let start_pos = wire_position(start_json, "lsp-range->offsets")?;
    let end_pos = wire_position(end_json, "lsp-range->offsets")?;
    let Some(lsp) = ctx.host.lsp() else {
        return Ok(SteelVal::BoolV(false));
    };
    let (Some(start), Some(end)) = (
        lsp.lsp_wire_to_char(bid, start_pos, encoding),
        lsp.lsp_wire_to_char(bid, end_pos, encoding),
    ) else {
        return Ok(SteelVal::BoolV(false));
    };
    Ok(symbol_hash([
        ("start", SteelVal::IntV(start as isize)),
        ("end", SteelVal::IntV(end as isize)),
    ]))
}

/// `(lsp-label-offsets->text label offsets)` → the slice of `label` that a
/// `ParameterInformation.label` `[start, end)` wire offset pair names, or
/// `#f` if this host has no LSP capability at all.
///
/// `offsets` is the raw two-element `[start, end)` array straight off the
/// wire, handed over undecoded the way `goto-location!` takes a raw
/// `Location`: either a Steel list (the ordinary hashmap-decode shape) or a
/// JSON array handle, both routed through `json_arg` onto the one
/// JSON-native decode below. Its encoding is read from its own tag: it is
/// itself drawn from the same response `label` came from (`json-ref`), so it
/// carries the producing server's negotiated encoding, and errors on an
/// untagged (hand-built) value, since there is then no server to have
/// negotiated one with. The offsets count code units in that encoding, so
/// Scheme can neither convert them nor index by them. That is the whole
/// reason this builtin exists rather than a Scheme helper.
pub(crate) fn lsp_label_offsets_to_text(
    ctx: &mut SteelCtx,
    label: SteelVal,
    offsets: SteelVal,
) -> SteelResult {
    let label = string_arg(label, "lsp-label-offsets->text label")?;
    let handle = json_arg(offsets, "lsp-label-offsets->text offsets")?;
    let encoding = handle
        .position_encoding("lsp-label-offsets->text")
        .map_err(generic_err)?;
    let arr = handle.value().as_array().ok_or_else(|| {
        generic_err("lsp-label-offsets->text: offsets must be a two-element (start end) array")
    })?;
    let [start, end] = arr.as_slice() else {
        return Err(generic_err(format!(
            "lsp-label-offsets->text: offsets must be a two-element (start end) array, got {} element(s)",
            arr.len()
        )));
    };
    let start = json_usize(start, "lsp-label-offsets->text offsets")?;
    let end = json_usize(end, "lsp-label-offsets->text offsets")?;
    Ok(match ctx.host.lsp() {
        Some(lsp) => SteelVal::StringV(
            lsp.lsp_label_offsets_to_text(&label, start, end, encoding)
                .into(),
        ),
        None => SteelVal::BoolV(false),
    })
}

/// `(lsp-locations->display-parts locs)` → one `(hash 'path p 'line l
/// 'grapheme-col-or-wire c 'buffer b 'location loc)` per distinct entry in
/// `locs` (rows naming the same place collapse into the first), a list of raw
/// `Location`/`LocationLink` hashmaps/handles: the display-side
/// counterpart to `goto-location!`'s wire conversion, decoded through the
/// same shared decoder. Each entry reads its own tagged producing-server
/// encoding (see `LspHost::lsp_locations_display_parts`'s doc). The column
/// is an exact grapheme column when the target has an open buffer, `#f`
/// when it's an open buffer whose line is out of range, and otherwise the
/// location's own wire `character` verbatim; this function never reads a
/// target file to refine that last case. `path`/`line` are always present,
/// since they come from the location itself. `buffer` is a buffer-only
/// pane handle for the open buffer the target is, `#f` when the file is not
/// open. See
/// `LspHost::lsp_locations_display_parts`'s doc for the full column-unit
/// rule and why a location that can't be decoded (or isn't tagged) at all
/// aborts the whole call rather than producing a degraded entry.
pub(crate) fn lsp_locations_to_display_parts(ctx: &mut SteelCtx, locs: SteelVal) -> SteelResult {
    let handles = list_items(locs, "lsp-locations->display-parts locs")?
        .into_iter()
        .map(|entry| json_arg(entry, "lsp-locations->display-parts locs"))
        .collect::<Result<Vec<_>, _>>()?;
    let parts = ctx
        .host
        .lsp()
        .ok_or_else(|| generic_err("lsp-locations->display-parts: no LSP state available"))?
        .lsp_locations_display_parts(&handles)
        .map_err(generic_err)?;
    let entries: Vec<SteelVal> = parts
        .into_iter()
        .map(|part| {
            symbol_hash([
                ("path", SteelVal::StringV(part.path.into())),
                ("line", SteelVal::IntV(part.line as isize)),
                (
                    "grapheme-col-or-wire",
                    match part.grapheme_col_or_wire {
                        Some(c) => SteelVal::IntV(c as isize),
                        None => SteelVal::BoolV(false),
                    },
                ),
                (
                    "buffer",
                    match part.buffer {
                        Some(bid) => SteelPane(PaneHandle::buffer_only(bid)).into_steel_val(),
                        None => SteelVal::BoolV(false),
                    },
                ),
                ("location", part.location.into_steel_val()),
            ])
        })
        .collect();
    Ok(SteelVal::ListV(entries.into()))
}

#[cfg(test)]
mod tests;
