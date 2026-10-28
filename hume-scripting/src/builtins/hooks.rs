//! `(register-hook! 'hook-name proc)` builtin.

use steel::rvals::SteelVal;

use super::SteelResult;
use super::args::{callable_arg, closure_shape, symbol_arg, takes_args, token_arg};
use super::errors::generic_err;
use crate::SteelCtx;
use crate::host::EventShape;

/// Decode a Steel event name: a symbol, validated against the host's
/// `known_event_names()`, with what that event hands its handlers.
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
/// (`ctx.host.events()`) is the sole authority on which events exist and
/// what each passes.
pub(crate) fn event_name_arg(
    ctx: &mut SteelCtx,
    val: &SteelVal,
    ctx_name: &str,
) -> Result<(String, EventShape), steel::rerrs::SteelErr> {
    let name_str = symbol_arg(val, ctx_name, "an event-name symbol like 'on-buffer-save")?;
    let shape = known_event(ctx, name_str).map_err(|e| generic_err(format!("{ctx_name}: {e}")))?;
    Ok((name_str.to_owned(), shape))
}

/// What the event `name` hands its handlers; errors unless the host knows
/// the event.
pub(crate) fn known_event(ctx: &mut SteelCtx, name: &str) -> Result<EventShape, String> {
    let events = ctx.host.events();
    events.event_shape(name).ok_or_else(|| {
        format!(
            "unknown event '{name}'; known events: {}",
            events.known_event_names().join(", ")
        )
    })
}

/// `(register-hook! 'name proc)`: register `proc` as a handler for the
/// named hook.  Must be called during init or plugin load (`EvalMode::Init`,
/// `PluginLoad`, or `PluginActivation`).
///
/// `on-language-set` fires `(lambda (pane lang) …)` on every language
/// transition, `lang` `""` for no language. For lazy-loaded language
/// plugins the typical pattern is:
/// `#:languages '("lang")` in `declare-plugin!` *activates* the body on the
/// first matching transition; the body then calls `(register-hook! 'on-language-set …)`
/// to install a *hook* that reacts on every subsequent transition.
pub(crate) fn register_hook(ctx: &mut SteelCtx, name: SteelVal, proc: SteelVal) -> SteelResult {
    let (name_str, shape) = event_name_arg(ctx, &name, "name")?;
    let proc = callable_arg(proc, "proc")?;
    register_entry(ctx, &name_str, shape, proc, None)?;
    Ok(SteelVal::Void)
}

/// Checks that `proc` can take what the event `name` hands it, and returns
/// whether it takes `done`. A queued event's handler is called with the
/// event's args. A builtin's parameter count can't be read, so Steel checks
/// it when the handler runs. A held event's handler is a plain lambda
/// taking the event's args, plus `done` when it wants to hold the raiser.
/// Its parameter count is how the two are told apart, so anything whose
/// count can't be read is refused.
fn handler_takes_done(
    name: &str,
    shape: EventShape,
    proc: &SteelVal,
) -> Result<bool, steel::rerrs::SteelErr> {
    let args = shape.args;
    if !shape.held {
        if takes_args(proc, args) {
            return Ok(false);
        }
        steel::stop!(TypeMismatch => "an '{} handler is a lambda of {} parameters", name, args);
    }
    match closure_shape(proc) {
        Some((count, false)) if count == args => Ok(false),
        Some((count, false)) if count == args + 1 => Ok(true),
        _ => steel::stop!(
            TypeMismatch =>
            "an '{} handler is a lambda of {} parameters, or {} to take `done`",
            name,
            args,
            args + 1
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
/// `register-lsp-notification-hook!`: the handler runs as the code that
/// registered it, and leaves with the plugin body running then if that body
/// fails. `proc` is checked against `shape`, what event `name` hands it.
pub(crate) fn register_entry(
    ctx: &mut SteelCtx,
    name: &str,
    shape: EventShape,
    proc: crate::Rooted,
    keys: Option<Box<[String]>>,
) -> Result<(), steel::rerrs::SteelErr> {
    let takes_done = handler_takes_done(name, shape, proc.value())?;
    let handler = crate::Callback::capture(ctx, proc);
    let activation = ctx.activation();
    ctx.registries
        .hooks
        .register(name, handler, activation, keys, takes_done);
    Ok(())
}

#[cfg(test)]
mod tests;
