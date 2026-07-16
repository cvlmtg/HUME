//! `(register-hook! 'hook-name proc)` builtin.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::hooks::HookId;

type SteelResult = Result<SteelVal, SteelErr>;

/// `(register-hook! 'name proc)` — register `proc` as a handler for the
/// named hook.  Must be called during init or plugin load (`EvalMode::Init`,
/// `PluginLoad`, or `PluginActivation`).
///
/// `name` must be a symbol matching one of the known hook names:
/// `on-buffer-open`, `on-buffer-close`, `on-buffer-save`, `on-mode-change`,
/// `on-language-set`.
///
/// `on-language-set` fires `(lambda (bid lang-or-#f) …)` on every language
/// transition.  For lazy-loaded language plugins the typical pattern is:
/// `#:languages '("lang")` in `declare-plugin` *activates* the body on the
/// first matching transition; the body then calls `(register-hook! 'on-language-set …)`
/// to install a *hook* that reacts on every subsequent transition.
pub(crate) fn register_hook(ctx: &mut SteelCtx, name: SteelVal, proc: SteelVal) -> SteelResult {
    let name_str = match &name {
        SteelVal::SymbolV(s) => s.to_string(),
        _ => steel::stop!(TypeMismatch => "register-hook!: expected a symbol, got {:?}", name),
    };
    let hook_id = match HookId::from_symbol(&name_str) {
        Some(id) => id,
        None => steel::stop!(
            Generic =>
            "register-hook!: unknown hook '{}'; known hooks: {}",
            name_str,
            HookId::all_names().collect::<Vec<_>>().join(", ")
        ),
    };
    let owner = ctx.plugin_stack.current().cloned();
    ctx.registries.hooks.register(hook_id, owner, proc);
    Ok(SteelVal::Void)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::HookId;
    use crate::test_support::SteelCtxTestHarness;

    /// `register-hook!` is blocked in plain command mode (init/plugin-load only).
    ///
    /// Fail oracle: change `register-hook!`'s table entry from `config` to
    /// `open` → the hook is silently registered from a command body, allowing
    /// plugins to change global behaviour at runtime.
    #[test]
    fn register_hook_blocked_in_command_mode() {
        let mut h = SteelCtxTestHarness::new();
        let result = super::super::errors::require_config(&h.ctx(), "register-hook!"); // EvalMode::Command
        assert!(result.is_err(), "register-hook! must error in command mode");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("init"),
            "error must mention 'init'; got: {msg}"
        );
    }

    /// `register-hook!` errors when the first argument is not a symbol.
    ///
    /// Fail oracle: remove the symbol check → strings or integers would be silently
    /// accepted, and the hook name lookup would silently fail.
    #[test]
    fn register_hook_non_symbol_arg_errors() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        let result = register_hook(
            &mut ctx,
            SteelVal::StringV("on-buffer-open".into()), // should be a symbol
            SteelVal::BoolV(true),
        );
        assert!(result.is_err(), "register-hook! must reject a string name");
    }

    /// `register-hook!` errors for an unknown hook name.
    ///
    /// Fail oracle: remove the HookId lookup guard → typos silently register
    /// a hook that is never fired.
    #[test]
    fn register_hook_unknown_hook_name_errors() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init();
        let result = register_hook(
            &mut ctx,
            SteelVal::SymbolV("on-nonexistent-event".into()),
            SteelVal::BoolV(true),
        );
        assert!(
            result.is_err(),
            "register-hook! must reject unknown hook names"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("unknown hook"),
            "error must mention 'unknown hook'; got: {msg}"
        );
    }

    /// `register-hook!` in init mode with a valid name registers the handler.
    ///
    /// Fail oracle: make `register` a no-op → `handlers_for` returns empty slice →
    /// last assert fires.
    #[test]
    fn register_hook_valid_in_init_mode() {
        let mut h = SteelCtxTestHarness::new();
        {
            let mut ctx = h.ctx_init();
            let result = register_hook(
                &mut ctx,
                SteelVal::SymbolV("on-buffer-save".into()),
                SteelVal::BoolV(true), // dummy proc — registry just stores SteelVal
            );
            assert!(result.is_ok(), "register-hook! must succeed in init mode");
        }
        assert_eq!(
            h.registries.hooks.handlers_for(HookId::OnBufferSave).len(),
            1,
            "one handler must be registered for on-buffer-save"
        );
        assert!(
            h.registries.hooks.handlers_for(HookId::OnBufferSave)[0]
                .owner
                .is_none(),
            "a top-level (non-plugin) registration must have no owner"
        );
    }

    /// `register-hook!` is also valid during plugin activation (plugin_stack non-empty).
    #[test]
    fn register_hook_valid_during_plugin_load() {
        use crate::attribution::PluginId;
        let mut h = SteelCtxTestHarness::new();
        // Simulate being inside a plugin body.
        h.plugin_stack
            .push(PluginId::parse("core:myplugin").unwrap());
        {
            let mut ctx = h.ctx(); // EvalMode::PluginActivation → allowed
            let result = register_hook(
                &mut ctx,
                SteelVal::SymbolV("on-buffer-open".into()),
                SteelVal::IntV(42),
            );
            assert!(
                result.is_ok(),
                "register-hook! must succeed during plugin load"
            );
        }
        assert!(!h.registries.hooks.is_empty_for(HookId::OnBufferOpen));
        assert_eq!(
            h.registries.hooks.handlers_for(HookId::OnBufferOpen)[0].owner,
            Some(PluginId::parse("core:myplugin").unwrap()),
            "a plugin-body registration must be attributed to the currently-executing plugin"
        );
    }
}
