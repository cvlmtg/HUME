//! LSP server lifecycle (register/unregister/stop/restart/status), the
//! generic request/notify bridge, and read-only introspection. Decorations,
//! completion, edit/navigation primitives, and the minibuffer prompt live in
//! their own modules. LSP is a client of those, not their owner.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::json::json_to_steel;
use crate::types::{
    Effect, LspServerTarget, PaneHandle, PendingLspNotify, PendingLspRequest, PendingLspServerOp,
};
use crate::{PendingLspServerReg, SteelCtx};

use super::SteelResult;
use super::args::{
    ArgPane, bool_arg, cons_pair, json_arg, json_params, list_items, list_to_env_pairs,
    list_to_strings, optional_json_arg, optional_string_arg, string_arg, wire_position,
};
use super::errors::generic_err;

/// `Some(json)` → decoded to a Steel hashmap; `None` (unresolvable, no
/// attached server, handshake incomplete, …) → `#f`. Shared by the three
/// params-builder introspection builtins below. Every field in these is
/// HUME-computed, not server JSON, so they stay a native decode; see
/// `lsp_capabilities`, which instead converts via `to_steel_handle` since
/// its own JSON isn't HUME-computed.
fn json_or_false(json: Option<serde_json::Value>) -> SteelVal {
    match json {
        Some(json) => json_to_steel(&json),
        None => SteelVal::BoolV(false),
    }
}

/// `(%register-lsp-server! language command args root-markers init-options settings env)`
///
/// Callable from init.scm, plugin activation, or a command/hook body,
/// unlike `%define-language!`, this is not gated to init/activation-only.
/// Queues a last-wins registration: applied at the end of the *current*
/// eval (see `Editor::apply_lsp_server_op`), replacing any existing
/// registration for `language` and attaching already-open matching buffers.
/// `lsp-registered-for-language?` reads through the effect log, so it
/// reports this registration as live immediately, within the same eval.
///
/// `args`/`root-markers` are lists of strings; `env` is a list of
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
    language: SteelVal,
    command: SteelVal,
    args_val: SteelVal,
    root_markers_val: SteelVal,
    init_options: SteelVal,
    settings: SteelVal,
    env_val: SteelVal,
) -> SteelResult {
    let language = string_arg(language, "register-lsp-server! language")?;
    let command = string_arg(command, "register-lsp-server! command")?;
    let args = list_to_strings(args_val, "register-lsp-server! args")?;
    let root_markers = list_to_strings(root_markers_val, "register-lsp-server! root-markers")?;
    let init_options = optional_json_arg(init_options, "register-lsp-server! init-options")?;
    let settings = optional_json_arg(settings, "register-lsp-server! settings")?;
    let env = list_to_env_pairs(env_val, "register-lsp-server! env")?;

    ctx.push_effect(Effect::LspServerOp(PendingLspServerOp::Register(
        PendingLspServerReg {
            language,
            command,
            args,
            root_markers,
            init_options,
            settings,
            env,
        },
    )));
    Ok(SteelVal::Void)
}

/// `(unregister-lsp-server! language)`: queues removal of `language`'s
/// registration and shutdown of any running clients for it, applied at the
/// end of the current eval (see `Editor::apply_lsp_server_op`).
///
/// Idempotent: unregistering a language with no registration and/or no
/// running clients is not an error: `:lsp-uninstall` of an already-removed
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
    ctx.push_effect(Effect::LspServerOp(PendingLspServerOp::Unregister {
        language,
    }));
    Ok(SteelVal::Void)
}

/// `(lsp-stop! target)`: `target` a buffer-id (that buffer's attached
/// server) or a string/symbol (every server registered for that language).
/// Queues a stop, applied at the end of the current eval (see
/// `Editor::apply_lsp_server_op`); the report of how many servers stopped is
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

/// `(%lsp-request pane method params callback allow-stale supersede
/// require-focus)`, behind the `lsp-request` wrapper (BOOTSTRAP), which
/// supplies the keyword defaults. Pushes an `Effect::LspRequest` that
/// `Editor::send_one_lsp_request` sends after this eval, since `SteelCtx` has
/// no route to the transport. The server is resolved from `pane`'s buffer at
/// that point, never from live focus, so a follow-up request from a callback
/// targets the original buffer.
///
/// `#:require-focus` fires the callback only if `pane` is still focused when
/// the response arrives: for cursor-anchored UI (hover, signature help, code
/// actions), not background requests. It raises at once if `pane` carries no
/// pane.
// Each param is a positional/keyword arg the `builtins!` table maps 1:1 from
// `lsp-request`'s own Steel signature, same rationale as
// `register_lsp_server`'s own `#[allow]`, just above in this file.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lsp_request(
    ctx: &mut SteelCtx,
    pane: PaneHandle,
    method: SteelVal,
    params: SteelVal,
    callback: SteelVal,
    allow_stale: SteelVal,
    supersede: SteelVal,
    require_focus: SteelVal,
) -> SteelResult {
    let method = string_arg(method, "lsp-request method")?;
    let params = json_params(params, "lsp-request params")?;
    let allow_stale = bool_arg(allow_stale, "lsp-request #:allow-stale")?;
    let supersede = optional_string_arg(supersede, "lsp-request supersede")?;
    let require_focus = bool_arg(require_focus, "lsp-request #:require-focus")?;
    let require_focus = require_focus
        .then(|| {
            pane.pane().ok_or_else(|| {
                generic_err("lsp-request: #:require-focus needs a pane, but was given none")
            })
        })
        .transpose()?;
    ctx.push_effect(Effect::LspRequest(PendingLspRequest {
        bid: pane.buffer(),
        method,
        params,
        callback,
        allow_stale,
        supersede,
        require_focus,
    }));
    Ok(SteelVal::Void)
}

/// `(lsp-notify pane method params)`: fire-and-forget, no callback, no
/// staleness tag (nothing to correlate a response against). Same queue
/// discipline as `lsp-request`, including `pane`'s buffer resolution
/// contract.
pub(crate) fn lsp_notify(
    ctx: &mut SteelCtx,
    pane: PaneHandle,
    method: SteelVal,
    params: SteelVal,
) -> SteelResult {
    let method = string_arg(method, "lsp-notify method")?;
    let params = json_params(params, "lsp-notify params")?;
    ctx.push_effect(Effect::LspNotify(PendingLspNotify {
        bid: pane.buffer(),
        method,
        params,
    }));
    Ok(SteelVal::Void)
}

/// `(on-lsp-notification method handler)`: registers `handler` for every
/// server notification of `method` that Rust doesn't already special-case
/// (window/logMessage, window/showMessage, $/progress, publishDiagnostics).
/// Persistent, immediate registration straight onto `ctx.registries`, same
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

/// `(lsp-capabilities pane)` → an opaque `JsonHandle` onto `pane`'s buffer's
/// attached server's wire `ServerCapabilities`, or `#f` if none is attached
/// or the handshake hasn't finished. Read it with `json-ref`/`json-contains?`.
pub(crate) fn lsp_capabilities(ctx: &mut SteelCtx, pane: ArgPane) -> SteelResult {
    Ok(ctx
        .host
        .lsp()
        .and_then(|lsp| lsp.lsp_capabilities(pane.0.buffer()))
        .map_or(SteelVal::BoolV(false), |v| {
            crate::json::to_steel_handle(v, crate::json::WireOrigin::Local)
        }))
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

/// `(lsp-server-for-buffer pane)` → registered language name, or `#f`.
pub(crate) fn lsp_server_for_buffer(ctx: &mut SteelCtx, pane: ArgPane) -> SteelResult {
    let id = pane.0.buffer();
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
/// live registry. The last queued `Register`/`Unregister` for `language`
/// wins, matching `Editor::apply_lsp_server_op`'s own last-wins semantics
/// exactly, so a same-eval registration is visible immediately instead of
/// only after the next drain.
///
/// Unlike its buffer/pane-touching siblings, this is a pure registry read
/// (no `EditorHost` state beyond the LSP registry itself), so its table
/// entry is `open` kind: no gate, callable during init/plugin load too.
/// That lets `core:lsp`'s own load-time scan (`registration.scm`) query it
/// directly to skip already-registered languages.
pub(crate) fn lsp_registered_for_language(ctx: &mut SteelCtx, language: SteelVal) -> SteelResult {
    let language = string_arg(language, "lsp-registered-for-language? language")?;
    let mut pending: Option<bool> = None;
    for queued in ctx.effects.iter() {
        let Effect::LspServerOp(op) = &queued.effect else {
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

/// Adapts a kind-B `LspHost` params method's `Result<Option<Value>, String>`
/// (`Err` on a stale `pane`, `Ok(None)` on no path/no attached server) to a
/// `SteelResult`: `Ok(None)` becomes `#f`, matching `json_or_false`'s own
/// convention for the "unavailable" case every one of these three builtins
/// shares.
fn params_result(result: Option<Result<Option<serde_json::Value>, String>>) -> SteelResult {
    match result {
        None => Ok(SteelVal::BoolV(false)),
        Some(Err(e)) => Err(generic_err(e)),
        Some(Ok(json)) => Ok(json_or_false(json)),
    }
}

/// `(lsp-position-params pane)` → `{"textDocument" {"uri"} "position" {"line"
/// "character"}}` from the primary cursor head in `pane`'s own pane, or `#f`
/// if unavailable (no attached server or no path). Raises (kind-B fail-fast)
/// when `pane` carries no pane, a closed one, or one that no longer shows
/// its buffer.
pub(crate) fn lsp_position_params(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    params_result(ctx.host.lsp().map(|lsp| lsp.lsp_position_params(pane)))
}

/// `(lsp-primary-range-params pane)` → same shape but a `"range"` from the
/// primary selection alone.
pub(crate) fn lsp_primary_range_params(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    params_result(ctx.host.lsp().map(|lsp| lsp.lsp_primary_range_params(pane)))
}

/// `(lsp-linewise-ranges-params pane)` → `{"textDocument" {"uri"} "ranges"
/// [...]}`, one wire range per linewise selection in `pane`'s own pane
/// (touching selections coalesced into one range apiece). Carries no
/// all/none/mixed verdict. `:lsp-fmt` gets that from `selections-linewise?`/
/// `selections-charwise?` instead, since every `lsp-*-params` builtin's
/// return value is a wire-ready params hash forwarded to `lsp-request`
/// unchanged or with a protocol key inserted, and a non-protocol key here
/// would break that (see `CursorHost::selections_linewise`'s doc comment).
pub(crate) fn lsp_linewise_ranges_params(ctx: &mut SteelCtx, pane: PaneHandle) -> SteelResult {
    params_result(
        ctx.host
            .lsp()
            .map(|lsp| lsp.lsp_linewise_ranges_params(pane)),
    )
}

/// A non-negative JSON integer, for a field read directly off a
/// `serde_json::Value` rather than decoded through a `SteelVal` arg (see
/// `super::args::usize_arg`'s Steel-side counterpart).
fn json_usize(v: &serde_json::Value, ctx_name: &str) -> Result<usize, SteelErr> {
    v.as_u64().map(|n| n as usize).ok_or_else(|| {
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

/// `(lsp-range->offsets pane range)` → `(start . end)` half-open char
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
    cons_pair(SteelVal::IntV(start as isize), SteelVal::IntV(end as isize))
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

/// `(lsp-locations->display-parts locs)` → one `(path line
/// grapheme-col-or-wire)` list per entry in `locs`, a list of raw
/// `Location`/`LocationLink` hashmaps/handles: the display-side
/// counterpart to `goto-location!`'s wire conversion, decoded through the
/// same shared decoder. Each entry reads its own tagged producing-server
/// encoding (see `LspHost::lsp_locations_display_parts`'s doc). The column
/// is an exact grapheme column when the target has an open buffer, `#f`
/// when it's an open buffer whose line is out of range, and otherwise the
/// location's own wire `character` verbatim; this function never reads a
/// target file to refine that last case. `path`/`line` are always present,
/// since they come from the location itself. See
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
            SteelVal::ListV(
                vec![
                    SteelVal::StringV(part.path.into()),
                    SteelVal::IntV(part.line as isize),
                    match part.grapheme_col_or_wire {
                        Some(c) => SteelVal::IntV(c as isize),
                        None => SteelVal::BoolV(false),
                    },
                ]
                .into(),
            )
        })
        .collect();
    Ok(SteelVal::ListV(entries.into()))
}

#[cfg(test)]
mod tests;
