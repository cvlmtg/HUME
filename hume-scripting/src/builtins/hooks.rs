//! `(register-hook! 'hook-name proc)` builtin.

use steel::rvals::SteelVal;

use super::SteelResult;
use crate::SteelCtx;
use crate::hooks::HookId;

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
mod tests;
