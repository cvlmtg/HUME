//! LSP server registration Steel builtin.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::json::{json_to_steel, steel_to_json};
use crate::types::{PendingLspNotify, PendingLspRequest, PendingLspServerOp};
use crate::{PendingLspServerReg, SteelCtx};

use super::{conv_err, list_to_strings, require_cmd_ctx, require_config_ctx, string_arg};

type SteelResult = Result<SteelVal, SteelErr>;

/// A string arg that may be `#f` (absent) — `lsp-request`/`lsp-notify`'s
/// `server` parameter: a registered language name, or "the focused buffer's
/// attached server".
fn optional_string_arg(val: SteelVal, ctx_name: &str) -> Result<Option<String>, SteelErr> {
    match val {
        SteelVal::BoolV(false) => Ok(None),
        other => Ok(Some(string_arg(other, ctx_name)?)),
    }
}

/// Converts `val` to the wire-shaped JSON a request/notification `params`
/// (or `#:init-options`/`#:settings` blob) expects — always an object (or
/// array), never a bare scalar. Rejects a bool explicitly: `(lsp-position-
/// params bid)`/`(lsp-range-params bid)` return `#f` when `bid` has no
/// attached server or isn't shown in any pane, and callers pass that result
/// straight through — without this check it would silently reach the wire
/// as `params: false` instead of erroring at the boundary.
fn json_params(val: SteelVal, ctx_name: &str) -> Result<serde_json::Value, SteelErr> {
    if matches!(val, SteelVal::BoolV(_)) {
        steel::stop!(TypeMismatch => "{ctx_name}: expected a hashmap, got a boolean");
    }
    steel_to_json(&val).map_err(|e| super::conv_err(format!("{ctx_name}: {e}")))
}

/// A blob arg that may be `#f` (absent) or any Steel data convertible to
/// JSON (typically a hashmap built with `(hash …)`).
fn optional_json_arg(val: SteelVal, ctx_name: &str) -> Result<Option<serde_json::Value>, SteelErr> {
    match val {
        SteelVal::BoolV(false) => Ok(None),
        other => Ok(Some(json_params(other, ctx_name)?)),
    }
}

/// `(%register-lsp-server! language command args root-markers init-options settings)`
///
/// Callable from init.scm, plugin activation, or a command/hook body —
/// unlike `%define-language!`, this is not gated to init/activation-only.
/// Queues a last-wins registration: applied at the end of the *current*
/// eval (see `Editor::apply_lsp_server_ops`), replacing any existing
/// registration for `language` and attaching already-open matching buffers.
/// `lsp-registered-for-language?` reads through this queue, so it reports
/// this registration as live immediately, within the same eval.
///
/// All list args must be lists of strings. Pushes a
/// `PendingLspServerOp::Register` onto `ctx.pending_lsp_server_ops`.
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

    ctx.pending_lsp_server_ops
        .push(PendingLspServerOp::Register(PendingLspServerReg {
            language,
            command,
            args,
            root_markers,
            init_options,
            settings,
        }));
    Ok(SteelVal::Void)
}

/// `(unregister-lsp-server! language)` — queues removal of `language`'s
/// registration and shutdown of any running clients for it, applied at the
/// end of the current eval (see `Editor::apply_lsp_server_ops`).
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
    ctx.pending_lsp_server_ops
        .push(PendingLspServerOp::Unregister { language });
    Ok(SteelVal::Void)
}

/// `(lsp-stop! language)` — `language` a string, or `#f` for "the focused
/// buffer's attached server". Queues a stop, applied at the end of the
/// current eval (see `Editor::apply_lsp_server_ops`); the report of how many
/// servers stopped is emitted by that same drain.
pub(crate) fn lsp_stop(ctx: &mut SteelCtx, language: SteelVal) -> SteelResult {
    require_cmd_ctx!(ctx, "lsp-stop!");
    let language = optional_string_arg(language, "lsp-stop! language")?;
    ctx.pending_lsp_server_ops
        .push(PendingLspServerOp::Stop { language });
    Ok(SteelVal::Void)
}

/// `(lsp-restart! language)` — same argument shape as `lsp-stop!`. Queues a
/// stop-then-respawn, applied at the end of the current eval.
pub(crate) fn lsp_restart(ctx: &mut SteelCtx, language: SteelVal) -> SteelResult {
    require_cmd_ctx!(ctx, "lsp-restart!");
    let language = optional_string_arg(language, "lsp-restart! language")?;
    ctx.pending_lsp_server_ops
        .push(PendingLspServerOp::Restart { language });
    Ok(SteelVal::Void)
}

/// `(lsp-show-status!)` — queues opening the `[lsp-status]` read-only view,
/// applied at the end of the current eval.
pub(crate) fn lsp_show_status(ctx: &mut SteelCtx) -> SteelResult {
    require_cmd_ctx!(ctx, "lsp-show-status!");
    ctx.pending_lsp_server_ops
        .push(PendingLspServerOp::ShowStatus);
    Ok(SteelVal::Void)
}

/// `(%lsp-request server method params callback allow-stale)`. The
/// `lsp-request` Scheme wrapper (BOOTSTRAP) supplies `#:allow-stale`'s
/// default. Queues a `PendingLspRequest`, flushed and actually sent by
/// `Editor::flush_pending_lsp_requests` right after this eval returns —
/// `SteelCtx` has no route to the transport (crate fence), and queuing
/// keeps every LSP send on one chokepoint regardless of which eval kind
/// (command, hook, or a queued callback) triggered it.
pub(crate) fn lsp_request(
    ctx: &mut SteelCtx,
    server: SteelVal,
    method: SteelVal,
    params: SteelVal,
    callback: SteelVal,
    allow_stale: SteelVal,
    supersede: SteelVal,
) -> SteelResult {
    require_cmd_ctx!(ctx, "lsp-request");
    let server = optional_string_arg(server, "lsp-request server")?;
    let method = string_arg(method, "lsp-request method")?;
    let params = json_params(params, "lsp-request params")?;
    let allow_stale = match allow_stale {
        SteelVal::BoolV(b) => b,
        _ => steel::stop!(TypeMismatch => "lsp-request: #:allow-stale expected a bool"),
    };
    let supersede = optional_string_arg(supersede, "lsp-request supersede")?;
    ctx.pending_lsp_requests.push(PendingLspRequest {
        server,
        method,
        params,
        callback,
        allow_stale,
        supersede,
    });
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
    require_cmd_ctx!(ctx, "lsp-notify");
    let server = optional_string_arg(server, "lsp-notify server")?;
    let method = string_arg(method, "lsp-notify method")?;
    let params = json_params(params, "lsp-notify params")?;
    ctx.pending_lsp_notifies.push(PendingLspNotify {
        server,
        method,
        params,
    });
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
    require_config_ctx!(ctx, "on-lsp-notification");
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
    require_cmd_ctx!(ctx, "lsp-capabilities");
    let server = optional_string_arg(server, "lsp-capabilities server")?;
    Ok(match ctx.host.lsp_capabilities(server.as_deref()) {
        Some(json) => json_to_steel(&json),
        None => SteelVal::BoolV(false),
    })
}

/// `(lsp-server-status)` → list of `{"language" "root" "state" "pending"}`.
pub(crate) fn lsp_server_status(ctx: &mut SteelCtx) -> SteelResult {
    require_cmd_ctx!(ctx, "lsp-server-status");
    let entries: Vec<SteelVal> = ctx
        .host
        .lsp_server_status()
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
pub(crate) fn lsp_server_for_buffer(ctx: &mut SteelCtx, bid: SteelVal) -> SteelResult {
    require_cmd_ctx!(ctx, "lsp-server-for-buffer");
    let id = bid_arg(&bid, "lsp-server-for-buffer")?;
    Ok(match ctx.host.lsp_server_for_buffer(id) {
        Some(lang) => SteelVal::StringV(lang.into()),
        None => SteelVal::BoolV(false),
    })
}

/// `(lsp-registered-for-language? language)` → bool. Registry query for the
/// `on-language-set` missing-server hint: distinguishes "no server
/// registered for this language" from "registered but still starting"
/// (`lsp-server-for-buffer` reports *attachment*, which can't make that
/// distinction). Reads through `ctx.pending_lsp_server_ops` (queued this
/// eval/init, not yet applied) in queue order before falling back to the
/// live registry — the last queued `Register`/`Unregister` for `language`
/// wins, matching `Editor::apply_lsp_server_ops`'s own last-wins semantics
/// exactly, so a same-eval registration is visible immediately instead of
/// only after the next drain.
///
/// Unlike its buffer/pane-touching siblings, this is a pure registry read
/// (no `EditorHost` state beyond the LSP registry itself), so it carries no
/// `require_cmd_ctx!` gate — callable during init/plugin load too. That lets
/// `core:lsp`'s own load-time scan (`registration.scm`) query it directly to
/// skip already-registered languages.
pub(crate) fn lsp_registered_for_language(ctx: &mut SteelCtx, language: SteelVal) -> SteelResult {
    let language = string_arg(language, "lsp-registered-for-language? language")?;
    let mut pending: Option<bool> = None;
    for op in ctx.pending_lsp_server_ops.iter() {
        match op {
            PendingLspServerOp::Register(reg) if reg.language == language => pending = Some(true),
            PendingLspServerOp::Unregister { language: l } if *l == language => {
                pending = Some(false)
            }
            _ => {}
        }
    }
    Ok(SteelVal::BoolV(
        pending.unwrap_or_else(|| ctx.host.lsp_registered_for_language(&language)),
    ))
}

/// `(lsp-position-params bid)` → `{"textDocument" {"uri"} "position" {"line"
/// "character"}}` from `bid`'s primary cursor head, or `#f` if unavailable
/// (no attached server, no path, or not shown in any pane).
pub(crate) fn lsp_position_params(ctx: &mut SteelCtx, bid: SteelVal) -> SteelResult {
    require_cmd_ctx!(ctx, "lsp-position-params");
    let id = bid_arg(&bid, "lsp-position-params")?;
    Ok(match ctx.host.lsp_position_params(id) {
        Some(json) => json_to_steel(&json),
        None => SteelVal::BoolV(false),
    })
}

/// `(lsp-range-params bid)` → same shape but a `"range"` from the primary
/// selection.
pub(crate) fn lsp_range_params(ctx: &mut SteelCtx, bid: SteelVal) -> SteelResult {
    require_cmd_ctx!(ctx, "lsp-range-params");
    let id = bid_arg(&bid, "lsp-range-params")?;
    Ok(match ctx.host.lsp_range_params(id) {
        Some(json) => json_to_steel(&json),
        None => SteelVal::BoolV(false),
    })
}

/// `(viewport-range bid)` → `(list first-line last-line)` currently visible
/// for `bid` (the focused pane's if shown there, else the first pane showing
/// it), or `#f` if `bid` isn't open in any pane. Pane geometry, not LSP
/// state, but gated the same as its buffer/pane-touching siblings — it reads
/// live view state, which only exists at command dispatch, hook fire, or a
/// queued-call drain.
pub(crate) fn viewport_range(ctx: &mut SteelCtx, bid: SteelVal) -> SteelResult {
    require_cmd_ctx!(ctx, "viewport-range");
    let id = bid_arg(&bid, "viewport-range")?;
    Ok(match ctx.host.viewport_range(id) {
        Some((first, last)) => {
            let entries: Vec<SteelVal> =
                vec![SteelVal::IntV(first as isize), SteelVal::IntV(last as isize)];
            SteelVal::ListV(entries.into())
        }
        None => SteelVal::BoolV(false),
    })
}

fn bid_arg(val: &SteelVal, ctx_name: &str) -> Result<hume_engine::pipeline::BufferId, SteelErr> {
    super::ids::downcast_buffer_id(val).ok_or_else(|| {
        SteelErr::new(
            steel::rerrs::ErrorKind::TypeMismatch,
            format!("{ctx_name}: expected buffer-id"),
        )
    })
}

/// `(register-trigger-chars! source language chars)` — `chars` is a list of
/// 1-char strings, registered for exactly `(source, language)`. Callable
/// from any context, including command bodies and hook handlers —
/// completion/signature-help register a server's trigger characters from
/// inside an `on-lsp-attach` handler, which runs as plain command context
/// (no `is_init`/`plugin_stack` gate applies here, unlike `register-hook!` /
/// `on-lsp-notification`).
pub(crate) fn register_trigger_chars(
    ctx: &mut SteelCtx,
    source: SteelVal,
    language: SteelVal,
    chars: SteelVal,
) -> SteelResult {
    let source = string_arg(source, "register-trigger-chars! source")?;
    let language = string_arg(language, "register-trigger-chars! language")?;
    let chars = chars_arg(chars, "register-trigger-chars! chars")?;
    ctx.host.register_trigger_chars(source, language, chars);
    Ok(SteelVal::Void)
}

fn chars_arg(val: SteelVal, ctx_name: &str) -> Result<Vec<char>, SteelErr> {
    list_to_strings(val, ctx_name)?
        .into_iter()
        .map(|s| {
            let mut it = s.chars();
            match (it.next(), it.next()) {
                (Some(c), None) => Ok(c),
                _ => steel::stop!(Generic =>
                    "{}: each entry must be exactly one character, got {:?}", ctx_name, s),
            }
        })
        .collect()
}

// ── Decoration stores + diagnostics pull ───────────────────────────────

fn list_items(val: SteelVal, ctx_name: &str) -> Result<Vec<SteelVal>, SteelErr> {
    match val {
        SteelVal::ListV(list) => Ok(list.into_iter().collect()),
        _ => steel::stop!(TypeMismatch => "{}: expected a list", ctx_name),
    }
}

fn usize_arg(val: SteelVal, ctx_name: &str) -> Result<usize, SteelErr> {
    match val {
        SteelVal::IntV(n) if n >= 0 => Ok(n as usize),
        _ => steel::stop!(TypeMismatch => "{}: expected a non-negative integer", ctx_name),
    }
}

fn int_arg(val: SteelVal, ctx_name: &str) -> Result<i64, SteelErr> {
    match val {
        SteelVal::IntV(n) => Ok(n as i64),
        _ => steel::stop!(TypeMismatch => "{}: expected an integer", ctx_name),
    }
}

/// Errors unless `fields` has exactly `n` elements — the shared shape check
/// for every setter's fixed-arity entry list.
fn exact_fields(
    fields: Vec<SteelVal>,
    n: usize,
    ctx_name: &str,
    shape: &str,
) -> Result<Vec<SteelVal>, SteelErr> {
    if fields.len() != n {
        steel::stop!(Generic => "{}: each entry must be {}", ctx_name, shape);
    }
    Ok(fields)
}

/// `(set-inlay-hints! bid hints)` — `hints`: list of `(position text
/// 'before|'after)`, `position` a wire `{"line" "character"}` hashmap.
pub(crate) fn set_inlay_hints(ctx: &mut SteelCtx, bid: SteelVal, hints: SteelVal) -> SteelResult {
    require_cmd_ctx!(ctx, "set-inlay-hints!");
    let id = bid_arg(&bid, "set-inlay-hints!")?;
    let mut parsed = Vec::new();
    for entry in list_items(hints, "set-inlay-hints! hints")? {
        let fields = exact_fields(
            list_items(entry, "set-inlay-hints! hint entry")?,
            3,
            "set-inlay-hints!",
            "(position text 'before|'after)",
        )?;
        let mut fields = fields.into_iter();
        let position = fields.next().expect("len checked");
        let text = fields.next().expect("len checked");
        let before_or_after = fields.next().expect("len checked");
        let position_json = steel_to_json(&position)
            .map_err(|e| super::conv_err(format!("set-inlay-hints! position: {e}")))?;
        // Validated here, at the boundary, rather than left to the host
        // side's extraction — a malformed position must error loudly, not
        // silently drop the hint (host_impl.rs's `set_inlay_hints` treats
        // this shape as already guaranteed).
        let has_valid_position = position_json.get("line").is_some_and(|v| v.is_u64())
            && position_json.get("character").is_some_and(|v| v.is_u64());
        if !has_valid_position {
            steel::stop!(Generic =>
                "set-inlay-hints!: position must be a hashmap with numeric 'line' and 'character' keys, got {}",
                position_json
            );
        }
        let text = string_arg(text, "set-inlay-hints! text")?;
        let before = match &before_or_after {
            SteelVal::SymbolV(s) if s.as_str() == "before" => true,
            SteelVal::SymbolV(s) if s.as_str() == "after" => false,
            _ => {
                steel::stop!(Generic => "set-inlay-hints!: third element must be 'before or 'after")
            }
        };
        parsed.push((position_json, text, before));
    }
    ctx.host.set_inlay_hints(id, parsed);
    Ok(SteelVal::Void)
}

/// `(set-signs! source bid signs)` — `signs`: list of `(line text scope priority)`.
pub(crate) fn set_signs(
    ctx: &mut SteelCtx,
    source: SteelVal,
    bid: SteelVal,
    signs: SteelVal,
) -> SteelResult {
    require_cmd_ctx!(ctx, "set-signs!");
    let source = string_arg(source, "set-signs! source")?;
    let id = bid_arg(&bid, "set-signs!")?;
    let mut parsed = Vec::new();
    for entry in list_items(signs, "set-signs! signs")? {
        let fields = exact_fields(
            list_items(entry, "set-signs! entry")?,
            4,
            "set-signs!",
            "(line text scope priority)",
        )?;
        let mut fields = fields.into_iter();
        let line = usize_arg(fields.next().expect("len checked"), "set-signs! line")?;
        let text = string_arg(fields.next().expect("len checked"), "set-signs! text")?;
        let scope = string_arg(fields.next().expect("len checked"), "set-signs! scope")?;
        let priority = int_arg(fields.next().expect("len checked"), "set-signs! priority")?;
        parsed.push((line, text, scope, priority));
    }
    ctx.host.set_signs(source, id, parsed);
    Ok(SteelVal::Void)
}

/// `(set-virtual-lines! source bid lines)` — `lines`: list of `(line text)`
/// or `(line text scope)`.
pub(crate) fn set_virtual_lines(
    ctx: &mut SteelCtx,
    source: SteelVal,
    bid: SteelVal,
    lines: SteelVal,
) -> SteelResult {
    require_cmd_ctx!(ctx, "set-virtual-lines!");
    let source = string_arg(source, "set-virtual-lines! source")?;
    let id = bid_arg(&bid, "set-virtual-lines!")?;
    let mut parsed = Vec::new();
    for entry in list_items(lines, "set-virtual-lines! lines")? {
        let fields = list_items(entry, "set-virtual-lines! entry")?;
        if fields.len() != 2 && fields.len() != 3 {
            steel::stop!(Generic => "set-virtual-lines!: each entry must be (line text) or (line text scope)");
        }
        let mut fields = fields.into_iter();
        let line = usize_arg(
            fields.next().expect("len checked"),
            "set-virtual-lines! line",
        )?;
        let text = string_arg(
            fields.next().expect("len checked"),
            "set-virtual-lines! text",
        )?;
        let scope = fields
            .next()
            .map(|v| string_arg(v, "set-virtual-lines! scope"))
            .transpose()?;
        parsed.push((line, text, scope));
    }
    ctx.host.set_virtual_lines(source, id, parsed);
    Ok(SteelVal::Void)
}

/// `(set-inline-diagnostics! bid lines)` — `lines`: list of `(line text
/// scope)`, one owner per buffer (no `source` arg, unlike
/// `set-virtual-lines!` — the diagnostics plugin is the only client).
pub(crate) fn set_inline_diagnostics(
    ctx: &mut SteelCtx,
    bid: SteelVal,
    lines: SteelVal,
) -> SteelResult {
    require_cmd_ctx!(ctx, "set-inline-diagnostics!");
    let id = bid_arg(&bid, "set-inline-diagnostics!")?;
    let mut parsed = Vec::new();
    for entry in list_items(lines, "set-inline-diagnostics! lines")? {
        let fields = exact_fields(
            list_items(entry, "set-inline-diagnostics! entry")?,
            3,
            "set-inline-diagnostics!",
            "(line text scope)",
        )?;
        let mut fields = fields.into_iter();
        let line = usize_arg(
            fields.next().expect("len checked"),
            "set-inline-diagnostics! line",
        )?;
        let text = string_arg(
            fields.next().expect("len checked"),
            "set-inline-diagnostics! text",
        )?;
        let scope = string_arg(
            fields.next().expect("len checked"),
            "set-inline-diagnostics! scope",
        )?;
        parsed.push((line, text, scope));
    }
    ctx.host.set_inline_diagnostics(id, parsed);
    Ok(SteelVal::Void)
}

/// `(set-extra-highlights! source bid spans)` — `spans`: list of `(start end scope)`.
pub(crate) fn set_extra_highlights(
    ctx: &mut SteelCtx,
    source: SteelVal,
    bid: SteelVal,
    spans: SteelVal,
) -> SteelResult {
    require_cmd_ctx!(ctx, "set-extra-highlights!");
    let source = string_arg(source, "set-extra-highlights! source")?;
    let id = bid_arg(&bid, "set-extra-highlights!")?;
    let mut parsed = Vec::new();
    for entry in list_items(spans, "set-extra-highlights! spans")? {
        let fields = exact_fields(
            list_items(entry, "set-extra-highlights! entry")?,
            3,
            "set-extra-highlights!",
            "(start end scope)",
        )?;
        let mut fields = fields.into_iter();
        let start = usize_arg(
            fields.next().expect("len checked"),
            "set-extra-highlights! start",
        )?;
        let end = usize_arg(
            fields.next().expect("len checked"),
            "set-extra-highlights! end",
        )?;
        let scope = string_arg(
            fields.next().expect("len checked"),
            "set-extra-highlights! scope",
        )?;
        parsed.push((start, end, scope));
    }
    ctx.host.set_extra_highlights(source, id, parsed);
    Ok(SteelVal::Void)
}

/// `(%diagnostics-for-buffer bid severity range)` — the `diagnostics-for-buffer`
/// Scheme wrapper supplies `#:severity`/`#:range` defaults. `severity`: a
/// symbol or `#f`. `range`: a 2-element list `(start end)` or `#f` — a
/// dotted pair isn't usable here (steel-core 0.8.2's `Pair`/`car`/`cdr` are
/// crate-private, so a Rust builtin can't destructure one).
pub(crate) fn diagnostics_for_buffer(
    ctx: &mut SteelCtx,
    bid: SteelVal,
    severity: SteelVal,
    range: SteelVal,
) -> SteelResult {
    require_cmd_ctx!(ctx, "diagnostics-for-buffer");
    let id = bid_arg(&bid, "diagnostics-for-buffer")?;
    let floor = match severity {
        SteelVal::BoolV(false) => None,
        SteelVal::SymbolV(s) => Some(s.to_string()),
        SteelVal::StringV(s) => Some(s.to_string()),
        _ => {
            steel::stop!(TypeMismatch => "diagnostics-for-buffer: #:severity expected a symbol or #f")
        }
    };
    let range = match range {
        SteelVal::BoolV(false) => None,
        other => {
            let fields = exact_fields(
                list_items(other, "diagnostics-for-buffer #:range")?,
                2,
                "diagnostics-for-buffer",
                "(start end)",
            )?;
            let mut fields = fields.into_iter();
            let start = usize_arg(
                fields.next().expect("len checked"),
                "diagnostics-for-buffer range start",
            )?;
            let end = usize_arg(
                fields.next().expect("len checked"),
                "diagnostics-for-buffer range end",
            )?;
            Some((start, end))
        }
    };
    let entries = ctx
        .host
        .diagnostics_for_buffer(id, floor.as_deref(), range)
        .map_err(|e| conv_err(format!("diagnostics-for-buffer: {e}")))?;
    let list: Vec<SteelVal> = entries.iter().map(json_to_steel).collect();
    Ok(SteelVal::ListV(list.into()))
}

/// `(diagnostic-counts bid)` → `(errors . warnings)` — a genuine dotted
/// pair, built via steel-core's public `cons` (the only public pair API).
pub(crate) fn diagnostic_counts(ctx: &mut SteelCtx, bid: SteelVal) -> SteelResult {
    require_cmd_ctx!(ctx, "diagnostic-counts");
    let id = bid_arg(&bid, "diagnostic-counts")?;
    let (errors, warnings) = ctx.host.diagnostic_counts(id);
    let mut errors_val = SteelVal::IntV(errors as isize);
    let mut warnings_val = SteelVal::IntV(warnings as isize);
    steel::primitives::lists::cons(&mut errors_val, &mut warnings_val)
}

// ── Edit + navigation primitives ───────────────────────────────────────

/// `(line col)` — a 2-element list, not a `(line . col)` dotted pair; see
/// `set-inlay-hints!`'s analogous choice (steel-core 0.8.2's `Pair`/`car`/
/// `cdr` are crate-private, unreachable from a Rust builtin).
fn position_pair(val: SteelVal, ctx_name: &str) -> Result<(usize, usize), SteelErr> {
    let fields = exact_fields(list_items(val, ctx_name)?, 2, ctx_name, "(line col)")?;
    let mut it = fields.into_iter();
    let line = usize_arg(it.next().expect("len checked"), ctx_name)?;
    let col = usize_arg(it.next().expect("len checked"), ctx_name)?;
    Ok((line, col))
}

fn optional_gen_arg(val: SteelVal) -> Result<Option<u64>, SteelErr> {
    match val {
        SteelVal::BoolV(false) => Ok(None),
        SteelVal::IntV(n) if n >= 0 => Ok(Some(n as u64)),
        _ => steel::stop!(TypeMismatch => "expect-generation must be a non-negative integer or #f"),
    }
}

/// `(%apply-text-edits! bid edits expect-gen)` — `edits`: list of `((start-
/// line start-col) (end-line end-col) text)`, wire positions.
pub(crate) fn apply_text_edits(
    ctx: &mut SteelCtx,
    bid: SteelVal,
    edits: SteelVal,
    expect_gen: SteelVal,
) -> SteelResult {
    require_cmd_ctx!(ctx, "apply-text-edits!");
    let id = bid_arg(&bid, "apply-text-edits!")?;
    let expect_gen = optional_gen_arg(expect_gen)?;
    let mut parsed = Vec::new();
    for entry in list_items(edits, "apply-text-edits! edits")? {
        let fields = exact_fields(
            list_items(entry, "apply-text-edits! entry")?,
            3,
            "apply-text-edits!",
            "((start-line start-col) (end-line end-col) text)",
        )?;
        let mut it = fields.into_iter();
        let (start_line, start_char) =
            position_pair(it.next().expect("len checked"), "apply-text-edits! start")?;
        let (end_line, end_char) =
            position_pair(it.next().expect("len checked"), "apply-text-edits! end")?;
        let new_text = string_arg(it.next().expect("len checked"), "apply-text-edits! text")?;
        parsed.push((start_line, start_char, end_line, end_char, new_text));
    }
    ctx.host
        .apply_text_edits(id, parsed, expect_gen)
        .map(|()| SteelVal::Void)
        .map_err(conv_err)
}

/// `(%apply-workspace-edit! wsedit)` — `wsedit`: the decoded `WorkspaceEdit`
/// hashmap (JSON↔SteelVal shape). Returns the number of buffers modified;
/// the `apply-workspace-edit!` Scheme wrapper reports that count.
pub(crate) fn apply_workspace_edit(ctx: &mut SteelCtx, wsedit: SteelVal) -> SteelResult {
    require_cmd_ctx!(ctx, "apply-workspace-edit!");
    let json = steel_to_json(&wsedit).map_err(conv_err)?;
    let count = ctx.host.apply_workspace_edit(json).map_err(conv_err)?;
    Ok(SteelVal::IntV(count as isize))
}

/// `(goto-location! loc)` — `loc` is one of two shapes, dispatched here (not
/// in Scheme): a raw `Location`/`LocationLink` hashmap (wire
/// position, converted using the focused buffer's server encoding — correct
/// because the caller is that server's own response callback), or `(list
/// target line col)` with char-indexed `line`/`col` and `target` a `bid`, a
/// path string, or a `file://` URI string.
pub(crate) fn goto_location(ctx: &mut SteelCtx, loc: SteelVal) -> SteelResult {
    require_cmd_ctx!(ctx, "goto-location!");
    match &loc {
        SteelVal::HashMapV(_) => {
            let json = steel_to_json(&loc).map_err(conv_err)?;
            let (uri, range) = if let Some(uri) = json.get("targetUri") {
                (
                    uri,
                    json.get("targetSelectionRange")
                        .or_else(|| json.get("targetRange")),
                )
            } else {
                (
                    json.get("uri")
                        .ok_or_else(|| conv_err("goto-location!: missing uri"))?,
                    json.get("range"),
                )
            };
            let uri = uri
                .as_str()
                .ok_or_else(|| conv_err("goto-location!: uri must be a string"))?
                .to_string();
            let range = range.ok_or_else(|| conv_err("goto-location!: missing range"))?;
            let line = range
                .pointer("/start/line")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| conv_err("goto-location!: missing range.start.line"))?
                as usize;
            let character = range
                .pointer("/start/character")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| conv_err("goto-location!: missing range.start.character"))?
                as usize;
            ctx.host
                .goto_location_wire(uri, line, character)
                .map(|()| SteelVal::Void)
                .map_err(conv_err)
        }
        SteelVal::ListV(_) => {
            let fields = exact_fields(
                list_items(loc.clone(), "goto-location!")?,
                3,
                "goto-location!",
                "(target line col)",
            )?;
            let mut it = fields.into_iter();
            let target = it.next().expect("len checked");
            let line = usize_arg(it.next().expect("len checked"), "goto-location! line")?;
            let col = usize_arg(it.next().expect("len checked"), "goto-location! col")?;
            if let Some(bid) = super::ids::downcast_buffer_id(&target) {
                ctx.host
                    .goto_location_buffer(bid, line, col)
                    .map(|()| SteelVal::Void)
                    .map_err(conv_err)
            } else {
                let s = string_arg(target, "goto-location! target")?;
                ctx.host
                    .goto_location_path(s, line, col)
                    .map(|()| SteelVal::Void)
                    .map_err(conv_err)
            }
        }
        _ => steel::stop!(TypeMismatch =>
            "goto-location!: expected a Location hashmap or (list target line col)"),
    }
}

/// `(selection-spans-full-line? bid)`.
pub(crate) fn selection_spans_full_line(ctx: &mut SteelCtx, bid: SteelVal) -> SteelResult {
    require_cmd_ctx!(ctx, "selection-spans-full-line?");
    let id = bid_arg(&bid, "selection-spans-full-line?")?;
    Ok(SteelVal::BoolV(ctx.host.selection_spans_full_line(id)))
}

// ── Minibuffer prompt ───────────────────────────────────────────────────

/// `(%prompt! label prefill on-confirm)` — the `prompt!` Scheme wrapper
/// supplies `#:prefill`'s default. `on-confirm` fires exactly once, later
/// (queued, never inline) — with the confirmed text, or `#f` on cancel.
pub(crate) fn prompt(
    ctx: &mut SteelCtx,
    label: SteelVal,
    prefill: SteelVal,
    on_confirm: SteelVal,
) -> SteelResult {
    require_cmd_ctx!(ctx, "prompt!");
    let label = string_arg(label, "prompt! label")?;
    let prefill = string_arg(prefill, "prompt! prefill")?;
    ctx.host
        .ui()
        .ok_or_else(|| conv_err(crate::host::unsupported("prompt!")))?
        .prompt(label, prefill, on_confirm)
        .map(|()| SteelVal::Void)
        .map_err(conv_err)
}

/// `(symbol-under-cursor bid)`.
pub(crate) fn symbol_under_cursor(ctx: &mut SteelCtx, bid: SteelVal) -> SteelResult {
    require_cmd_ctx!(ctx, "symbol-under-cursor");
    let id = bid_arg(&bid, "symbol-under-cursor")?;
    Ok(SteelVal::StringV(ctx.host.symbol_under_cursor(id).into()))
}

// ── Completion orchestration ────────────────────────────────────────────

/// `(%completion-begin! bid items incomplete)` — the `completion-begin!`
/// Scheme wrapper supplies `#:incomplete`'s default. `items`: list of
/// decoded `CompletionItem` hashmaps.
pub(crate) fn completion_begin(
    ctx: &mut SteelCtx,
    bid: SteelVal,
    items: SteelVal,
    incomplete: SteelVal,
) -> SteelResult {
    require_cmd_ctx!(ctx, "completion-begin!");
    let id = bid_arg(&bid, "completion-begin!")?;
    let incomplete = match incomplete {
        SteelVal::BoolV(b) => b,
        _ => steel::stop!(TypeMismatch => "completion-begin!: #:incomplete expected a bool"),
    };
    let mut parsed = Vec::new();
    for entry in list_items(items, "completion-begin! items")? {
        parsed.push(steel_to_json(&entry).map_err(conv_err)?);
    }
    ctx.host
        .completion_begin(id, parsed, incomplete)
        .map(|()| SteelVal::Void)
        .map_err(conv_err)
}

/// `(completion-update-filter! text)`.
pub(crate) fn completion_update_filter(ctx: &mut SteelCtx, text: SteelVal) -> SteelResult {
    require_cmd_ctx!(ctx, "completion-update-filter!");
    let text = string_arg(text, "completion-update-filter! text")?;
    ctx.host
        .completion_update_filter(text)
        .map(|()| SteelVal::Void)
        .map_err(conv_err)
}

/// `(completion-top n)`.
pub(crate) fn completion_top(ctx: &mut SteelCtx, n: SteelVal) -> SteelResult {
    require_cmd_ctx!(ctx, "completion-top");
    let n = usize_arg(n, "completion-top")?;
    let items = ctx.host.completion_top(n);
    let list: Vec<SteelVal> = items.iter().map(json_to_steel).collect();
    Ok(SteelVal::ListV(list.into()))
}

/// `(completion-accept! idx)` — `idx` indexes the ranked/filtered list
/// (`completion-top`'s order), not the raw response order.
pub(crate) fn completion_accept(ctx: &mut SteelCtx, idx: SteelVal) -> SteelResult {
    require_cmd_ctx!(ctx, "completion-accept!");
    let idx = usize_arg(idx, "completion-accept!")?;
    ctx.host
        .completion_accept(idx)
        .map(|()| SteelVal::Void)
        .map_err(conv_err)
}

/// `(completion-dismiss!)`.
pub(crate) fn completion_dismiss(ctx: &mut SteelCtx) -> SteelResult {
    require_cmd_ctx!(ctx, "completion-dismiss!");
    ctx.host.completion_dismiss();
    Ok(SteelVal::Void)
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

    /// Unwraps the single queued op as a `Register`, panicking with a message
    /// naming the actual variant otherwise — so a misrouted `Unregister`
    /// fails loudly instead of silently indexing the wrong data.
    fn expect_register(h: &SteelCtxTestHarness) -> &crate::PendingLspServerReg {
        assert_eq!(h.pending_lsp_server_ops.len(), 1);
        match &h.pending_lsp_server_ops[0] {
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
        assert_eq!(h.pending_lsp_server_ops.len(), 1);
        match &h.pending_lsp_server_ops[0] {
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

        assert_eq!(h.pending_lsp_server_ops.len(), 3);
        assert!(matches!(
            &h.pending_lsp_server_ops[0],
            PendingLspServerOp::Register(reg) if reg.args.is_empty()
        ));
        assert!(matches!(
            &h.pending_lsp_server_ops[1],
            PendingLspServerOp::Unregister { language } if language == "rust"
        ));
        assert!(matches!(
            &h.pending_lsp_server_ops[2],
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
        assert_eq!(h.pending_lsp_server_ops.len(), 1);
        match &h.pending_lsp_server_ops[0] {
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
        match &h.pending_lsp_server_ops[0] {
            PendingLspServerOp::Stop { language } => assert_eq!(*language, None),
            other => panic!("expected Stop, got {other:?}"),
        }
    }

    #[test]
    fn lsp_stop_rejects_init_context() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        ctx.is_init = true;
        let err = lsp_stop(&mut ctx, SteelVal::BoolV(false)).unwrap_err();
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
        assert_eq!(h.pending_lsp_server_ops.len(), 1);
        match &h.pending_lsp_server_ops[0] {
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
        ctx.is_init = true;
        let err = lsp_restart(&mut ctx, SteelVal::BoolV(false)).unwrap_err();
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
        assert_eq!(h.pending_lsp_server_ops.len(), 1);
        assert!(matches!(
            &h.pending_lsp_server_ops[0],
            PendingLspServerOp::ShowStatus
        ));
    }

    #[test]
    fn lsp_show_status_rejects_init_context() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        ctx.is_init = true;
        let err = lsp_show_status(&mut ctx).unwrap_err();
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
    /// Fail oracle: reinstate `require_cmd_ctx!` in
    /// `lsp_registered_for_language` → this returns `Err` instead of `Ok`.
    #[test]
    fn lsp_registered_for_language_is_callable_during_init() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        ctx.is_init = true;
        let result = lsp_registered_for_language(&mut ctx, "rust".into_steelval().unwrap());
        assert_eq!(
            result.unwrap(),
            SteelVal::BoolV(false),
            "NullHost reports nothing registered"
        );
    }

    fn pending_register(language: &str) -> PendingLspServerOp {
        PendingLspServerOp::Register(crate::PendingLspServerReg {
            language: language.to_string(),
            command: "rust-analyzer".to_string(),
            args: Vec::new(),
            root_markers: Vec::new(),
            init_options: None,
            settings: None,
        })
    }

    fn pending_unregister(language: &str) -> PendingLspServerOp {
        PendingLspServerOp::Unregister {
            language: language.to_string(),
        }
    }

    /// R1: `lsp-registered-for-language?` reads through `ctx.pending_lsp_server_ops`
    /// before falling back to the host — a `Register` queued this eval must
    /// be visible immediately, not only after the next drain.
    #[test]
    fn a_queued_register_reports_true_within_the_same_eval() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        ctx.pending_lsp_server_ops.push(pending_register("rust"));
        let result = lsp_registered_for_language(&mut ctx, "rust".into_steelval().unwrap());
        assert_eq!(result.unwrap(), SteelVal::BoolV(true));
    }

    /// Queue order, not queue presence, decides the answer — a later
    /// `Unregister` overrides an earlier `Register` for the same language,
    /// matching `Editor::apply_lsp_server_ops`'s own last-wins application
    /// order exactly.
    #[test]
    fn register_then_unregister_in_queue_order_reports_false() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        ctx.pending_lsp_server_ops.push(pending_register("rust"));
        ctx.pending_lsp_server_ops.push(pending_unregister("rust"));
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
        ctx.pending_lsp_server_ops.push(pending_unregister("rust"));
        ctx.pending_lsp_server_ops.push(pending_register("rust"));
        let result = lsp_registered_for_language(&mut ctx, "rust".into_steelval().unwrap());
        assert_eq!(result.unwrap(), SteelVal::BoolV(true));
    }

    /// A queued op for a *different* language must not affect the answer.
    #[test]
    fn a_queued_op_for_a_different_language_does_not_flip_the_answer() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx();
        ctx.pending_lsp_server_ops.push(pending_register("python"));
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
        ctx.pending_lsp_server_ops
            .push(PendingLspServerOp::Stop {
                language: Some("rust".to_string()),
            });
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
        // `pending_lsp_requests` lives directly on `SteelCtx` (not the
        // harness's `HostBundle`), so it must be read before `ctx` drops.
        assert_eq!(ctx.pending_lsp_requests.len(), 1);
        assert_eq!(
            ctx.pending_lsp_requests[0].supersede,
            Some("completion".to_string())
        );
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
        assert_eq!(ctx.pending_lsp_requests.len(), 1);
        assert_eq!(ctx.pending_lsp_requests[0].supersede, None);
    }
}
