//! LSP server registration Steel builtin.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::{PendingLspServerReg, SteelCtx};

use super::list_to_strings;

type SteelResult = Result<SteelVal, SteelErr>;

fn string_arg(val: SteelVal, ctx_name: &str) -> Result<String, SteelErr> {
    match val {
        SteelVal::StringV(s) => Ok(s.to_string()),
        SteelVal::SymbolV(s) => Ok(s.to_string()),
        _ => steel::stop!(TypeMismatch => "{}: expected a string", ctx_name),
    }
}

/// A string arg that may be `#f` (absent). `init-options`/`settings` travel
/// as raw JSON text until B1 lands a real JSON<->SteelVal codec — the
/// editor glue parses them at flush time.
fn optional_json_string_arg(val: SteelVal, ctx_name: &str) -> Result<Option<String>, SteelErr> {
    match val {
        SteelVal::BoolV(false) => Ok(None),
        SteelVal::StringV(s) => Ok(Some(s.to_string())),
        _ => steel::stop!(TypeMismatch => "{}: expected a JSON string or #f", ctx_name),
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
    let init_options = optional_json_string_arg(init_options, "register-lsp-server! init-options")?;
    let settings = optional_json_string_arg(settings, "register-lsp-server! settings")?;

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
    fn carries_json_strings_through_untouched() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        let result = register_lsp_server(
            &mut ctx,
            "rust".into_steelval().unwrap(),
            "rust-analyzer".into_steelval().unwrap(),
            list_of(&[]),
            list_of(&[]),
            "{\"a\":1}".into_steelval().unwrap(),
            "{\"b\":2}".into_steelval().unwrap(),
        );
        assert!(result.is_ok());
        drop(ctx);
        let reg = &h.pending_lsp_server_regs[0];
        assert_eq!(reg.init_options.as_deref(), Some("{\"a\":1}"));
        assert_eq!(reg.settings.as_deref(), Some("{\"b\":2}"));
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
    fn invalid_init_options_type_is_a_type_mismatch_error() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        let result = register_lsp_server(
            &mut ctx,
            "rust".into_steelval().unwrap(),
            "rust-analyzer".into_steelval().unwrap(),
            list_of(&[]),
            list_of(&[]),
            SteelVal::IntV(1),
            SteelVal::BoolV(false),
        );
        assert!(result.is_err());
    }
}
