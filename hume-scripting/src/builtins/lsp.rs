//! LSP server lifecycle (register/unregister/stop/restart/status), the
//! generic request/notify bridge, and read-only introspection. Decorations,
//! completion, edit/navigation primitives, and the minibuffer prompt live in
//! their own modules — LSP is a client of those, not their owner.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::json::{json_to_steel, steel_to_json};
use crate::types::{Effect, PendingLspNotify, PendingLspRequest, PendingLspServerOp};
use crate::{PendingLspServerReg, SteelCtx};

use super::SteelResult;
use super::args::{
    BidArg, bool_arg, cons_pair, json_params, list_to_env_pairs, list_to_strings,
    optional_json_arg, optional_string_arg, string_arg,
};
use super::errors::generic_err;

/// `Some(json)` → decoded to a Steel hashmap; `None` (unresolvable, no
/// attached server, handshake incomplete, …) → `#f`. Shared by the three
/// introspection builtins below.
fn json_or_false(json: Option<serde_json::Value>) -> SteelVal {
    match json {
        Some(json) => json_to_steel(&json),
        None => SteelVal::BoolV(false),
    }
}

/// `(%register-lsp-server! language command args root-markers init-options settings env)`
///
/// Callable from init.scm, plugin activation, or a command/hook body —
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
// `register-lsp-server!`'s own Steel signature — bundling them into a struct
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
    ctx.push_effect(Effect::LspServerOp(PendingLspServerOp::Unregister {
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
    ctx.push_effect(Effect::LspServerOp(PendingLspServerOp::Stop { language }));
    Ok(SteelVal::Void)
}

/// `(lsp-restart! language)` — same argument shape as `lsp-stop!`. Queues a
/// stop-then-respawn, applied at the end of the current eval.
pub(crate) fn lsp_restart(ctx: &mut SteelCtx, language: SteelVal) -> SteelResult {
    let language = optional_string_arg(language, "lsp-restart! language")?;
    ctx.push_effect(Effect::LspServerOp(PendingLspServerOp::Restart {
        language,
    }));
    Ok(SteelVal::Void)
}

/// `(lsp-show-status!)` — queues opening the `[lsp-status]` read-only view,
/// applied at the end of the current eval.
pub(crate) fn lsp_show_status(ctx: &mut SteelCtx) -> SteelResult {
    ctx.push_effect(Effect::LspServerOp(PendingLspServerOp::ShowStatus));
    Ok(SteelVal::Void)
}

/// `(%lsp-request server method params callback allow-stale supersede)`. The
/// `lsp-request` Scheme wrapper (BOOTSTRAP) supplies `#:allow-stale`'s and
/// `#:supersede`'s defaults. Pushes an `Effect::LspRequest`, sent by `Editor::send_one_lsp_request`
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
    let allow_stale = bool_arg(allow_stale, "lsp-request #:allow-stale")?;
    let supersede = optional_string_arg(supersede, "lsp-request supersede")?;
    ctx.push_effect(Effect::LspRequest(PendingLspRequest {
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
    ctx.push_effect(Effect::LspNotify(PendingLspNotify {
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
    Ok(json_or_false(
        ctx.host
            .lsp()
            .and_then(|lsp| lsp.lsp_capabilities(server.as_deref())),
    ))
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

/// `(lsp-position-params bid)` → `{"textDocument" {"uri"} "position" {"line"
/// "character"}}` from `bid`'s primary cursor head, or `#f` if unavailable
/// (no attached server, no path, or not shown in any pane).
pub(crate) fn lsp_position_params(ctx: &mut SteelCtx, bid: BidArg) -> SteelResult {
    let id = bid.0;
    Ok(json_or_false(
        ctx.host.lsp().and_then(|lsp| lsp.lsp_position_params(id)),
    ))
}

/// `(lsp-range-params bid)` → same shape but a `"range"` from the primary
/// selection.
pub(crate) fn lsp_range_params(ctx: &mut SteelCtx, bid: BidArg) -> SteelResult {
    let id = bid.0;
    Ok(json_or_false(
        ctx.host.lsp().and_then(|lsp| lsp.lsp_range_params(id)),
    ))
}

/// Decodes a wire `{"line" "character"}` hashmap. `what` names the calling
/// builtin in the error message — `wire_to_char` (the eventual conversion)
/// is total and clamps rather than errors, so this boundary check is the
/// only place a malformed shape gets caught instead of silently producing a
/// plausible-looking offset.
fn wire_position(v: &serde_json::Value, what: &str) -> Result<(usize, usize), SteelErr> {
    match (
        v.get("line").and_then(serde_json::Value::as_u64),
        v.get("character").and_then(serde_json::Value::as_u64),
    ) {
        (Some(line), Some(character)) => Ok((line as usize, character as usize)),
        _ => Err(generic_err(format!(
            "{what}: position must be a hashmap with numeric 'line' and 'character' keys, got {v}"
        ))),
    }
}

/// `(lsp-position->offset bid position)` → `bid`'s char offset for the wire
/// `{"line" "character"}` hashmap `position`, converted using `bid`'s
/// attached server's negotiated encoding — or `#f` if `bid` has no attached
/// server (no negotiated encoding to convert with), or if `position` would
/// land on the buffer's trailing phantom line (a stale response racing an
/// edit, or a server's past-end convention) — every point-anchored
/// decoration setter (`set-inlay-hints!`) rejects that offset outright, so
/// refusing here lets a caller filter one bad entry instead of the whole
/// setter call failing on it.
pub(crate) fn lsp_position_to_offset(
    ctx: &mut SteelCtx,
    bid: BidArg,
    position: SteelVal,
) -> SteelResult {
    let id = bid.0;
    let position_json =
        steel_to_json(&position).map_err(|e| generic_err(format!("lsp-position->offset: {e}")))?;
    let (line, character) = wire_position(&position_json, "lsp-position->offset")?;
    Ok(
        match ctx
            .host
            .lsp()
            .and_then(|lsp| lsp.lsp_wire_point_to_char(id, line, character))
        {
            Some(offset) => SteelVal::IntV(offset as isize),
            None => SteelVal::BoolV(false),
        },
    )
}

/// `(lsp-range->offsets bid range)` → `(start . end)` half-open char offsets
/// for the wire `{"start" {"line" "character"} "end" {"line" "character"}}`
/// hashmap `range`, same encoding rule as `lsp-position->offset`. `#f` if
/// `bid` has no attached server.
pub(crate) fn lsp_range_to_offsets(
    ctx: &mut SteelCtx,
    bid: BidArg,
    range: SteelVal,
) -> SteelResult {
    let id = bid.0;
    let range_json =
        steel_to_json(&range).map_err(|e| generic_err(format!("lsp-range->offsets: {e}")))?;
    let start_json = range_json
        .get("start")
        .ok_or_else(|| generic_err("lsp-range->offsets: range missing 'start'"))?;
    let end_json = range_json
        .get("end")
        .ok_or_else(|| generic_err("lsp-range->offsets: range missing 'end'"))?;
    let (start_line, start_character) = wire_position(start_json, "lsp-range->offsets")?;
    let (end_line, end_character) = wire_position(end_json, "lsp-range->offsets")?;
    let Some(lsp) = ctx.host.lsp() else {
        return Ok(SteelVal::BoolV(false));
    };
    let (Some(start), Some(end)) = (
        lsp.lsp_wire_to_char(id, start_line, start_character),
        lsp.lsp_wire_to_char(id, end_line, end_character),
    ) else {
        return Ok(SteelVal::BoolV(false));
    };
    cons_pair(SteelVal::IntV(start as isize), SteelVal::IntV(end as isize))
}

#[cfg(test)]
mod tests;
