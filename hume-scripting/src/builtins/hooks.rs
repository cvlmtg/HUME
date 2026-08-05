//! `(register-hook! 'hook-name proc)` builtin.

use steel::rvals::SteelVal;

use super::SteelResult;
use crate::SteelCtx;

/// Decode a Steel event name: a symbol, validated against the host's
/// `known_event_names()`.
///
/// Shared by `register-hook!` and `declare-plugin`'s `#:events` — the two
/// verbs that name an event — so the accepted form and the error text can't
/// drift apart the way they used to (one took symbols only, the other took
/// strings or symbols).
///
/// Symbol, not string: the event set is closed and host-defined, same rule
/// as `bind-key!`'s mode argument. `#:commands` / `#:languages` stay
/// strings — those names are open and user-chosen, not host-enumerated.
///
/// This crate has no compiled-in list of event names of its own; the editor
/// (`ctx.host.events().known_event_names()`) is the sole authority on which
/// events exist.
pub(crate) fn event_name_arg(
    ctx: &mut SteelCtx,
    val: &SteelVal,
    verb: &str,
) -> Result<String, steel::rerrs::SteelErr> {
    let name_str = match val {
        SteelVal::SymbolV(s) => s.to_string(),
        _ => steel::stop!(TypeMismatch =>
            "{verb}: expected an event-name symbol like 'on-buffer-save, got {:?}", val),
    };
    let known = ctx.host.events().known_event_names();
    if !known.contains(&name_str.as_str()) {
        steel::stop!(
            Generic =>
            "{verb}: unknown event '{}'; known events: {}",
            name_str,
            known.join(", ")
        );
    }
    Ok(name_str)
}

/// `(register-hook! 'name proc)` — register `proc` as a handler for the
/// named hook.  Must be called during init or plugin load (`EvalMode::Init`,
/// `PluginLoad`, or `PluginActivation`).
///
/// `on-language-set` fires `(lambda (bid lang-or-#f) …)` on every language
/// transition.  For lazy-loaded language plugins the typical pattern is:
/// `#:languages '("lang")` in `declare-plugin` *activates* the body on the
/// first matching transition; the body then calls `(register-hook! 'on-language-set …)`
/// to install a *hook* that reacts on every subsequent transition.
pub(crate) fn register_hook(ctx: &mut SteelCtx, name: SteelVal, proc: SteelVal) -> SteelResult {
    let name_str = event_name_arg(ctx, &name, "register-hook!")?;
    let owner = ctx.plugin_stack.current().cloned();
    ctx.registries.hooks.register(&name_str, owner, proc);
    Ok(SteelVal::Void)
}

#[cfg(test)]
mod tests;
