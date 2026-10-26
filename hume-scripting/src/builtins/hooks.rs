//! `(register-hook! 'hook-name proc)` builtin.

use steel::rvals::SteelVal;

use super::SteelResult;
use super::args::{callable_arg, closure_shape, symbol_arg, token_arg};
use super::errors::generic_err;
use crate::SteelCtx;

/// Decode a Steel event name: a symbol, validated against the host's
/// `known_event_names()`.
///
/// Shared by `register-hook!` and `declare-plugin!`'s `#:events` (the two
/// verbs that name an event), so the accepted form and the error text can't
/// drift apart between them.
///
/// Symbol, not string: the event set is closed and host-defined, same rule
/// as `bind-key!`'s mode argument. `#:commands` / `#:typed-commands` /
/// `#:languages` stay strings: those names are open and user-chosen, not
/// host-enumerated.
///
/// This crate has no compiled-in list of event names of its own; the editor
/// (`ctx.host.events().known_event_names()`) is the sole authority on which
/// events exist.
pub(crate) fn event_name_arg(
    ctx: &mut SteelCtx,
    val: &SteelVal,
    ctx_name: &str,
) -> Result<String, steel::rerrs::SteelErr> {
    let name_str = symbol_arg(val, ctx_name, "an event-name symbol like 'on-buffer-save")?;
    known_event(ctx, name_str).map_err(|e| generic_err(format!("{ctx_name}: {e}")))?;
    Ok(name_str.to_owned())
}

/// Errors unless the host knows the event `name`.
pub(crate) fn known_event(ctx: &mut SteelCtx, name: &str) -> Result<(), String> {
    let known = ctx.host.events().known_event_names();
    if known.contains(&name) {
        Ok(())
    } else {
        Err(format!(
            "unknown event '{name}'; known events: {}",
            known.join(", ")
        ))
    }
}

/// `(register-hook! 'name proc)`: register `proc` as a handler for the
/// named hook.  Must be called during init or plugin load (`EvalMode::Init`,
/// `PluginLoad`, or `PluginActivation`).
///
/// `on-language-set` fires `(lambda (bid lang-or-#f) …)` on every language
/// transition.  For lazy-loaded language plugins the typical pattern is:
/// `#:languages '("lang")` in `declare-plugin!` *activates* the body on the
/// first matching transition; the body then calls `(register-hook! 'on-language-set …)`
/// to install a *hook* that reacts on every subsequent transition.
pub(crate) fn register_hook(ctx: &mut SteelCtx, name: SteelVal, proc: SteelVal) -> SteelResult {
    let name_str = event_name_arg(ctx, &name, "name")?;
    let proc = callable_arg(proc, "proc")?;
    register_entry(ctx, &name_str, proc, None)?;
    Ok(SteelVal::Void)
}

/// A handler of a held event is a plain lambda taking the event's args,
/// plus `done` when it wants to hold the raiser. Returns whether it takes
/// `done`. Its parameter count is how the two are told apart, so anything
/// whose count can't be read is refused.
fn held_handler_takes_done(
    name: &str,
    event_args: usize,
    proc: &SteelVal,
) -> Result<bool, steel::rerrs::SteelErr> {
    match closure_shape(proc) {
        Some((count, false)) if count == event_args => Ok(false),
        Some((count, false)) if count == event_args + 1 => Ok(true),
        _ => steel::stop!(
            TypeMismatch =>
            "a '{} handler is a lambda of {} parameters, or {} to take `done`",
            name,
            event_args,
            event_args + 1
        ),
    }
}

/// `(%held-done hold)`: what a held handler's `done` thunk calls to release
/// its hold.
pub(crate) fn held_done(ctx: &mut SteelCtx, hold: SteelVal) -> SteelResult {
    ctx.host.events().finish_hold(token_arg(hold, "done")?);
    Ok(SteelVal::Void)
}

/// The one registration path behind `register-hook!` and
/// `register-lsp-notification-hook!`: attributes the entry to the plugin
/// whose body is running, so rollback removes it with that plugin. A
/// handler of a held event is checked for whether it takes `done`.
pub(crate) fn register_entry(
    ctx: &mut SteelCtx,
    name: &str,
    proc: crate::Rooted,
    keys: Option<Box<[String]>>,
) -> Result<(), steel::rerrs::SteelErr> {
    let takes_done = match ctx.host.events().held_event_arg_count(name) {
        Some(event_args) => held_handler_takes_done(name, event_args, proc.value())?,
        None => false,
    };
    let owner = ctx.plugin_stack.current().cloned();
    ctx.registries
        .hooks
        .register(name, owner, proc, keys, takes_done);
    Ok(())
}

#[cfg(test)]
mod tests;
