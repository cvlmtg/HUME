//! `(register-hook! 'hook-name proc)` builtin.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;
use crate::hooks::HookId;

type SteelResult = Result<SteelVal, SteelErr>;

/// `(register-hook! 'name proc)` — register `proc` as a handler for the
/// named hook.  Must be called during init / plugin load (`is_init = true`).
///
/// `name` must be a symbol matching one of the known hook names:
/// `on-buffer-open`, `on-buffer-close`, `on-buffer-save`, `on-mode-change`,
/// `on-language-set`.
///
/// `on-language-set` fires `(lambda (bid lang-or-#f) …)` on every language
/// transition.  For lazy-loaded language plugins the typical pattern is:
/// `#:on-language '("lang")` in `declare-plugin` activates the body on the
/// first matching transition; the body then calls `(register-hook! 'on-language-set …)`
/// to react on all subsequent transitions.
pub(crate) fn register_hook(ctx: &mut SteelCtx, name: SteelVal, proc: SteelVal) -> SteelResult {
    if !ctx.is_init && ctx.plugin_stack.is_empty() {
        steel::stop!(Generic => "register-hook!: can only be called during init/plugin load");
    }
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
    ctx.registries.hooks.register(hook_id, proc);
    Ok(SteelVal::Void)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::SteelCtxTestHarness;
    use crate::hooks::HookId;

    /// `register-hook!` is blocked in plain command mode (init/plugin-load only).
    ///
    /// Fail oracle: remove the is_init guard → the hook is silently registered
    /// from a command body, allowing plugins to change global behaviour at runtime.
    #[test]
    fn register_hook_blocked_in_command_mode() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx(); // is_init=false, plugin_stack empty
        let result = register_hook(
            &mut ctx,
            SteelVal::SymbolV("on-buffer-open".into()),
            SteelVal::BoolV(true),
        );
        assert!(result.is_err(), "register-hook! must error in command mode");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("init"), "error must mention 'init'; got: {msg}");
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
        assert!(result.is_err(), "register-hook! must reject unknown hook names");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("unknown hook"), "error must mention 'unknown hook'; got: {msg}");
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
    }

    /// `register-hook!` is also valid during plugin activation (plugin_stack non-empty).
    #[test]
    fn register_hook_valid_during_plugin_load() {
        use crate::attribution::PluginId;
        let mut h = SteelCtxTestHarness::new();
        // Simulate being inside a plugin body.
        h.plugin_stack.push(PluginId::parse("core:myplugin").unwrap());
        {
            let mut ctx = h.ctx(); // is_init=false but plugin_stack non-empty → allowed
            let result = register_hook(
                &mut ctx,
                SteelVal::SymbolV("on-buffer-open".into()),
                SteelVal::IntV(42),
            );
            assert!(result.is_ok(), "register-hook! must succeed during plugin load");
        }
        assert!(!h.registries.hooks.is_empty_for(HookId::OnBufferOpen));
    }
}
