//! LSP server registration Steel builtin.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::json::{json_to_steel, steel_to_json};
use crate::types::{PendingLspNotify, PendingLspRequest};
use crate::{PendingLspServerReg, SteelCtx};

use super::{list_to_strings, require_cmd_ctx};

type SteelResult = Result<SteelVal, SteelErr>;

fn string_arg(val: SteelVal, ctx_name: &str) -> Result<String, SteelErr> {
    match val {
        SteelVal::StringV(s) => Ok(s.to_string()),
        SteelVal::SymbolV(s) => Ok(s.to_string()),
        _ => steel::stop!(TypeMismatch => "{}: expected a string", ctx_name),
    }
}

/// A string arg that may be `#f` (absent) — `lsp-request`/`lsp-notify`'s
/// `server` parameter: a registered language name, or "the focused buffer's
/// attached server".
fn optional_string_arg(val: SteelVal, ctx_name: &str) -> Result<Option<String>, SteelErr> {
    match val {
        SteelVal::BoolV(false) => Ok(None),
        other => Ok(Some(string_arg(other, ctx_name)?)),
    }
}

fn json_params(val: SteelVal, ctx_name: &str) -> Result<serde_json::Value, SteelErr> {
    steel_to_json(&val).map_err(|e| super::conv_err(format!("{ctx_name}: {e}")))
}

/// A blob arg that may be `#f` (absent) or any Steel data convertible to
/// JSON (typically a hashmap built with `(hash …)`).
fn optional_json_arg(val: SteelVal, ctx_name: &str) -> Result<Option<serde_json::Value>, SteelErr> {
    match val {
        SteelVal::BoolV(false) => Ok(None),
        other => match steel_to_json(&other) {
            Ok(json) => Ok(Some(json)),
            Err(msg) => steel::stop!(TypeMismatch => "{}: {}", ctx_name, msg),
        },
    }
}

/// `(%register-lsp-server! language command args root-markers init-options settings)`
/// — init-only.
///
/// All list args must be lists of strings. Pushes a `PendingLspServerReg`
/// onto `ctx.pending_lsp_server_regs`; `Editor::flush_pending_lsp_server_regs`
/// applies them once init.scm finishes (same queueing shape as
/// `%define-language!`).
pub(crate) fn register_lsp_server(
    ctx: &mut SteelCtx,
    language: SteelVal,
    command: SteelVal,
    args_val: SteelVal,
    root_markers_val: SteelVal,
    init_options: SteelVal,
    settings: SteelVal,
) -> SteelResult {
    if !ctx.is_init && ctx.plugin_stack.is_empty() {
        steel::stop!(Generic => "%register-lsp-server!: only callable during init (use register-lsp-server! in init.scm or a plugin)");
    }
    let language = string_arg(language, "register-lsp-server! language")?;
    let command = string_arg(command, "register-lsp-server! command")?;
    let args = list_to_strings(args_val, "register-lsp-server! args")?;
    let root_markers = list_to_strings(root_markers_val, "register-lsp-server! root-markers")?;
    let init_options = optional_json_arg(init_options, "register-lsp-server! init-options")?;
    let settings = optional_json_arg(settings, "register-lsp-server! settings")?;

    ctx.pending_lsp_server_regs.push(PendingLspServerReg {
        language,
        command,
        args,
        root_markers,
        init_options,
        settings,
    });
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
) -> SteelResult {
    require_cmd_ctx!(ctx, "lsp-request");
    let server = optional_string_arg(server, "lsp-request server")?;
    let method = string_arg(method, "lsp-request method")?;
    let params = json_params(params, "lsp-request params")?;
    let allow_stale = match allow_stale {
        SteelVal::BoolV(b) => b,
        _ => steel::stop!(TypeMismatch => "lsp-request: #:allow-stale expected a bool"),
    };
    ctx.pending_lsp_requests.push(PendingLspRequest {
        server,
        method,
        params,
        callback,
        allow_stale,
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
    if !ctx.is_init && ctx.plugin_stack.is_empty() {
        steel::stop!(Generic => "on-lsp-notification: only callable during init/plugin load");
    }
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
            map.insert(SteelVal::StringV("language".into()), SteelVal::StringV(e.language.into()));
            map.insert(
                SteelVal::StringV("root".into()),
                SteelVal::StringV(e.root.to_string_lossy().into_owned().into()),
            );
            map.insert(SteelVal::StringV("state".into()), SteelVal::StringV(e.state.into()));
            map.insert(SteelVal::StringV("pending".into()), SteelVal::IntV(e.pending as isize));
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

fn bid_arg(val: &SteelVal, ctx_name: &str) -> Result<hume_engine::pipeline::BufferId, SteelErr> {
    super::ids::downcast_buffer_id(val)
        .ok_or_else(|| SteelErr::new(steel::rerrs::ErrorKind::TypeMismatch, format!("{ctx_name}: expected buffer-id")))
}

/// `(register-trigger-chars! source chars)` — `chars` is a list of 1-char
/// strings. Same init/plugin-load gate as `register-hook!` / `on-lsp-notification`.
pub(crate) fn register_trigger_chars(
    ctx: &mut SteelCtx,
    source: SteelVal,
    chars: SteelVal,
) -> SteelResult {
    if !ctx.is_init && ctx.plugin_stack.is_empty() {
        steel::stop!(Generic => "register-trigger-chars!: only callable during init/plugin load");
    }
    let source = string_arg(source, "register-trigger-chars! source")?;
    let chars = chars_arg(chars, "register-trigger-chars! chars")?;
    ctx.host.register_trigger_chars(source, chars);
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

// ── B5: decoration stores + diagnostics pull ───────────────────────────────

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

/// Pops exactly `n` elements off the end of `fields` in reverse (so the
/// returned `Vec` is in original left-to-right order), erroring if the
/// count doesn't match — the shared shape check for every setter's
/// fixed-arity entry list.
fn exact_fields(fields: Vec<SteelVal>, n: usize, ctx_name: &str, shape: &str) -> Result<Vec<SteelVal>, SteelErr> {
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
        let text = string_arg(text, "set-inlay-hints! text")?;
        let before = match &before_or_after {
            SteelVal::SymbolV(s) if s.as_str() == "before" => true,
            SteelVal::SymbolV(s) if s.as_str() == "after" => false,
            _ => steel::stop!(Generic => "set-inlay-hints!: third element must be 'before or 'after"),
        };
        parsed.push((position_json, text, before));
    }
    ctx.host.set_inlay_hints(id, parsed);
    Ok(SteelVal::Void)
}

/// `(set-signs! source bid signs)` — `signs`: list of `(line text scope priority)`.
pub(crate) fn set_signs(ctx: &mut SteelCtx, source: SteelVal, bid: SteelVal, signs: SteelVal) -> SteelResult {
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

/// `(set-virtual-lines! source bid lines)` — `lines`: list of `(line text)`.
pub(crate) fn set_virtual_lines(ctx: &mut SteelCtx, source: SteelVal, bid: SteelVal, lines: SteelVal) -> SteelResult {
    require_cmd_ctx!(ctx, "set-virtual-lines!");
    let source = string_arg(source, "set-virtual-lines! source")?;
    let id = bid_arg(&bid, "set-virtual-lines!")?;
    let mut parsed = Vec::new();
    for entry in list_items(lines, "set-virtual-lines! lines")? {
        let fields = exact_fields(
            list_items(entry, "set-virtual-lines! entry")?,
            2,
            "set-virtual-lines!",
            "(line text)",
        )?;
        let mut fields = fields.into_iter();
        let line = usize_arg(fields.next().expect("len checked"), "set-virtual-lines! line")?;
        let text = string_arg(fields.next().expect("len checked"), "set-virtual-lines! text")?;
        parsed.push((line, text));
    }
    ctx.host.set_virtual_lines(source, id, parsed);
    Ok(SteelVal::Void)
}

/// `(set-extra-highlights! source bid spans)` — `spans`: list of `(start end scope)`.
pub(crate) fn set_extra_highlights(ctx: &mut SteelCtx, source: SteelVal, bid: SteelVal, spans: SteelVal) -> SteelResult {
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
        let start = usize_arg(fields.next().expect("len checked"), "set-extra-highlights! start")?;
        let end = usize_arg(fields.next().expect("len checked"), "set-extra-highlights! end")?;
        let scope = string_arg(fields.next().expect("len checked"), "set-extra-highlights! scope")?;
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
        _ => steel::stop!(TypeMismatch => "diagnostics-for-buffer: #:severity expected a symbol or #f"),
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
            let start = usize_arg(fields.next().expect("len checked"), "diagnostics-for-buffer range start")?;
            let end = usize_arg(fields.next().expect("len checked"), "diagnostics-for-buffer range end")?;
            Some((start, end))
        }
    };
    let entries = ctx.host.diagnostics_for_buffer(id, floor.as_deref(), range);
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
        assert_eq!(h.pending_lsp_server_regs.len(), 1);
        let reg = &h.pending_lsp_server_regs[0];
        assert_eq!(reg.language, "rust");
        assert_eq!(reg.command, "rust-analyzer");
        assert_eq!(reg.root_markers, vec!["Cargo.toml".to_string()]);
        assert_eq!(reg.init_options, None);
        assert_eq!(reg.settings, None);
    }

    #[test]
    fn decodes_steel_hashmap_blobs_to_json() {
        use steel::gc::Gc;
        use steel::HashMap as SteelHashMap;

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
        let reg = &h.pending_lsp_server_regs[0];
        assert_eq!(reg.init_options, Some(serde_json::json!({"a": 1})));
        assert_eq!(reg.settings, Some(serde_json::json!({"b": 2})));
    }

    /// Fail oracle: call outside init (command mode, empty plugin stack) →
    /// the guard must fire and nothing gets queued.
    #[test]
    fn rejected_outside_init_and_plugin_activation() {
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
        assert!(result.is_err());
        drop(ctx);
        assert!(h.pending_lsp_server_regs.is_empty());
    }

    #[test]
    fn allowed_during_plugin_activation_even_though_is_init_is_false() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_activation();
        ctx.plugin_stack.push(crate::PluginId::Core("lsp".to_string()));
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
}
