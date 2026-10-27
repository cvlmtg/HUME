//! The generic request/notify bridge and read-only server introspection;
//! server registration and lifecycle are in `servers`, request params in
//! `params`, wire/editor conversions in `convert`. Decorations, completion,
//! edit/navigation primitives, and the minibuffer prompt live in their own
//! modules. LSP is a client of those, not their owner.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::types::{
    CapabilityQuery, Effect, LspFeature, PaneHandle, Params, PendingLspNotify, PendingLspRequest,
    PendingLspServerOp, RequestMode, RequestParams, RouteSpec, WhenUnavailable,
};
use crate::{Callback, SERVER_REGISTRATION_TIE, SteelCtx};

use super::SteelResult;
use super::args::{
    bool_arg, callable_arg, checked_params, list_of, optional_server_arg, optional_string_arg,
    optional_supersede_key_arg, optional_symbol_arg, optional_token_arg, pair_fields, server_arg,
    string_arg, string_list, strings_arg, symbol_enum_arg, symbol_hash,
};
use super::errors::{generic_err, require_cap};
use super::hooks::{known_event, register_entry};
use super::ids::ServerRef;
use super::wire::tagged_json_arg;

mod convert;
mod params;
mod servers;

pub(super) use convert::{
    lsp_label_offsets_to_text, lsp_locations_to_merged_places, lsp_locations_to_places,
    lsp_position_to_char_offset, lsp_range_to_char_range,
};
pub(super) use params::{
    keep_tracked_position, lsp_document_params, lsp_linewise_ranges_params, lsp_position_params,
    lsp_primary_range_params, track_position, tracked_position_params, untrack_position,
};
use servers::server_name_arg;
pub(super) use servers::{
    LspTargetArg, lsp_language_servers, lsp_restart, lsp_show_status, lsp_stop,
    register_lsp_server, set_default_language_servers, set_language_servers, unregister_lsp_server,
};

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
fn route_arg(feature: SteelVal, to: SteelVal) -> Result<RouteSpec, SteelErr> {
    Ok(RouteSpec {
        feature: feature_arg(feature, "#:feature")?,
        to: optional_server_arg(to, "#:to")?,
    })
}

/// A `#:feature` names the feature of a method the editor has none for. A
/// standard method already has one, and a second could only disagree.
fn reject_feature_on_standard_method(
    ctx: &mut SteelCtx,
    route: &RouteSpec,
    method: &str,
) -> Result<(), SteelErr> {
    if route.feature.is_none() {
        return Ok(());
    }
    match require_cap(ctx.host.lsp())?.lsp_method_feature(method) {
        Some(feature) => Err(generic_err(format!(
            "{method} is a standard method of the '{} feature; #:feature is for other methods",
            feature.name()
        ))),
        None => Ok(()),
    }
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
    callback: Callback,
    unavailable: SteelVal,
    allow_stale: SteelVal,
    supersede: SteelVal,
    require_focus: SteelVal,
    tracked: SteelVal,
) -> SteelResult {
    let method = string_arg(method, "method")?;
    reject_feature_on_standard_method(ctx, &route, &method)?;
    let when_unavailable = optional_symbol_arg(unavailable, "#:unavailable")?
        .map(|name| symbol_enum_arg(&name, "#:unavailable", &WhenUnavailable::NAMED))
        .transpose()?
        .unwrap_or_default();
    let allow_stale = bool_arg(allow_stale, "#:allow-stale")?;
    let supersede = optional_supersede_key_arg(supersede, "#:supersede")?;
    let require_focus = bool_arg(require_focus, "#:require-focus")?
        .then(|| {
            pane.pane()
                .ok_or_else(|| generic_err("#:require-focus needs a pane, but was given none"))
        })
        .transpose()?;
    let tracked = match optional_token_arg(tracked, "#:tracked")? {
        Some(token) => require_cap(ctx.host.lsp())?.hold_tracked_position(token),
        None => None,
    };
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
    callback: Callback,
    feature: SteelVal,
    to: SteelVal,
    unavailable: SteelVal,
    allow_stale: SteelVal,
    supersede: SteelVal,
    require_focus: SteelVal,
    tracked: SteelVal,
) -> SteelResult {
    let route = route_arg(feature, to)?;
    let params = RequestParams::Shared(checked_params(params, "params")?);
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
    callback: Callback,
    feature: SteelVal,
    unavailable: SteelVal,
    allow_stale: SteelVal,
    supersede: SteelVal,
    require_focus: SteelVal,
    tracked: SteelVal,
) -> SteelResult {
    const CTX: &str = "params";
    let route = route_arg(feature, SteelVal::BoolV(false))?;
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
    let method = string_arg(method, "method")?;
    let params = checked_params(params, "params")?;
    let route = route_arg(feature, to)?;
    reject_feature_on_standard_method(ctx, &route, &method)?;
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
    let feature = feature_arg(feature, "#:feature")?;
    let method = optional_string_arg(method, "#:method")?;
    let servers = require_cap(ctx.host.lsp())?
        .lsp_servers(pane.buffer(), feature, method.as_deref())
        .map_err(generic_err)?;
    Ok(list_of(servers.into_iter().map(ServerRef::into_steel_val)))
}

/// `(lsp-server-name server)` → the name `server` was registered under.
pub(crate) fn lsp_server_name(server: SteelVal) -> SteelResult {
    let server = server_arg(&server, "server")?;
    Ok(SteelVal::StringV(server.name.as_str().into()))
}

/// `(lsp-location uri range)` → a `Location` handle pairing the `uri` string
/// with `range`, a range taken from a server response. The result carries the
/// range's tag, so `goto-location!` and `lsp-locations->places` decode
/// it with the encoding of the server that sent the range. A hand-built range
/// has no encoding to carry and raises.
pub(crate) fn lsp_location(uri: SteelVal, range: SteelVal) -> SteelResult {
    let uri = string_arg(uri, "uri")?;
    let (range, _) = tagged_json_arg(&range, "range")?;
    let location = serde_json::json!({"uri": uri, "range": range.value()});
    Ok(crate::json::to_steel_handle(
        std::sync::Arc::new(location),
        range.origin(),
    ))
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
    let methods = strings_arg(methods, "methods")?;
    if methods.is_empty() {
        steel::stop!(Generic => "methods must not be empty");
    }
    let proc = callable_arg(proc, "proc")?;
    known_event(ctx, LSP_NOTIFICATION_EVENT).map_err(generic_err)?;
    register_entry(
        ctx,
        LSP_NOTIFICATION_EVENT,
        proc,
        Some(methods.into_boxed_slice()),
    )?;
    Ok(SteelVal::Void)
}

/// `(lsp-capabilities server)` → an opaque `JsonHandle` onto `server`'s
/// wire `ServerCapabilities`, or `#f` once it has stopped. Read it with
/// `json-ref`/`json-contains?`.
pub(crate) fn lsp_capabilities(ctx: &mut SteelCtx, server: SteelVal) -> SteelResult {
    let server = server_arg(&server, "server")?;
    Ok(require_cap(ctx.host.lsp())?
        .lsp_capabilities(server.id)
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
    let server = server_arg(&server, "server")?;
    let feature = feature_arg(feature, "#:feature")?;
    let method = optional_string_arg(method, "#:method")?;
    let query = match (feature, method.as_deref()) {
        (Some(feature), None) => CapabilityQuery::Feature(feature),
        (None, Some(method)) => CapabilityQuery::Method(method),
        _ => {
            return Err(generic_err(
                "give exactly one of #:feature and #:method".to_string(),
            ));
        }
    };
    Ok(require_cap(ctx.host.lsp())?
        .lsp_capability(server.id, query)
        .map_or(SteelVal::BoolV(false), |v| {
            crate::json::to_steel_handle(v, crate::json::WireOrigin::Local)
        }))
}

/// `(lsp-server-status)` → list of `(hash 'name 'languages 'root 'state 'pending)`.
pub(crate) fn lsp_server_status(ctx: &mut SteelCtx) -> SteelResult {
    let entries: Vec<SteelVal> = require_cap(ctx.host.lsp())?
        .lsp_server_status()
        .into_iter()
        .map(|e| {
            symbol_hash([
                ("name", SteelVal::StringV(e.name.as_str().into())),
                ("languages", string_list(e.languages)),
                (
                    "root",
                    SteelVal::StringV(
                        hume_platform::path::strip_unc_prefix(e.root)
                            .to_string_lossy()
                            .into_owned()
                            .into(),
                    ),
                ),
                ("state", SteelVal::SymbolV(e.state.name().into())),
                ("pending", SteelVal::IntV(e.pending as isize)),
            ])
        })
        .collect();
    Ok(SteelVal::ListV(entries.into()))
}

/// `(lsp-supersede-key)`: a new `#:supersede` key, equal only to itself.
/// `open` kind, so a plugin mints its keys once at its top level.
pub(crate) fn lsp_supersede_key(ctx: &mut SteelCtx) -> SteelResult {
    Ok(require_cap(ctx.host.lsp())?
        .new_supersede_key()
        .into_steel_val())
}

/// `(lsp-server-registered? name)` → bool. Reads through the
/// `Effect::LspServerOp` entries queued this eval, in emission order, from
/// the live registry's registration, by the rule
/// `Editor::apply_lsp_server_ops` applies them by
/// (`ConfigOrigin::rank` with [`SERVER_REGISTRATION_TIE`]), so a same-eval
/// registration is visible at once and an op that will not apply is not. A host with no LSP capability
/// raises whether or not an op is queued.
///
/// A pure registry read, so its table entry is `open` kind: callable during
/// init/plugin load too, which lets `core:lsp-install`'s load-time scan skip
/// names already registered.
pub(crate) fn lsp_server_registered(ctx: &mut SteelCtx, name: SteelVal) -> SteelResult {
    let name = server_name_arg(name, "name")?;
    let held = require_cap(ctx.host.lsp())?.lsp_server_origin(&name);
    let held = ctx
        .effects
        .iter()
        .fold(held, |held, queued| match &queued.effect {
            Effect::LspServerOp(PendingLspServerOp::Register(reg))
                if reg.name == name
                    && reg.origin.rank(held, SERVER_REGISTRATION_TIE).is_applied() =>
            {
                Some(reg.origin)
            }
            Effect::LspServerOp(PendingLspServerOp::Unregister { name: n, origin })
                if *n == name && origin.rank(held, SERVER_REGISTRATION_TIE).is_applied() =>
            {
                None
            }
            _ => held,
        });
    Ok(SteelVal::BoolV(held.is_some()))
}

#[cfg(test)]
mod tests;
