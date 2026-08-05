//! `(register-hook! 'hook-name proc)` builtin.

use steel::rvals::SteelVal;

use super::SteelResult;
use crate::SteelCtx;

/// `(register-hook! 'name proc)` — register `proc` as a handler for the
/// named hook.  Must be called during init or plugin load (`EvalMode::Init`,
/// `PluginLoad`, or `PluginActivation`).
///
/// `name` must be a symbol matching one of the host's known event names
/// (`ctx.host.events().known_event_names()`) — this crate has no compiled-in
/// list of its own; the editor is the authority on which events exist.
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
    let known = ctx.host.events().known_event_names();
    if !known.contains(&name_str.as_str()) {
        steel::stop!(
            Generic =>
            "register-hook!: unknown hook '{}'; known hooks: {}",
            name_str,
            known.join(", ")
        );
    }
    let owner = ctx.plugin_stack.current().cloned();
    ctx.registries.hooks.register(&name_str, owner, proc);
    Ok(SteelVal::Void)
}

#[cfg(test)]
mod tests;
