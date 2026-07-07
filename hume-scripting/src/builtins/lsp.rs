//! LSP server registration Steel builtin.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::json::steel_to_json;
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
