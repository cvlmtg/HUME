//! One error idiom for builtins: `steel::stop!` for early returns,
//! [`generic_err`] for `map_err`/closure positions where an early return
//! doesn't fit. Plus the two `EvalMode` gate checks every config/command
//! builtin opens with.
//!
//! No other module constructs a `SteelErr` directly — enforced by
//! `rg 'SteelErr::new' hume-scripting/src/builtins --glob '!args.rs' --glob '!errors.rs'`
//! returning empty.

use steel::rerrs::{ErrorKind, SteelErr};

use crate::context::{EvalMode, SteelCtx};

/// Map a host-layer failure (or any other `Display`-able error) into a Steel
/// `Generic` error. For `map_err`/closure positions, where `steel::stop!`'s
/// early-return doesn't apply.
pub(crate) fn generic_err(msg: impl std::fmt::Display) -> SteelErr {
    SteelErr::new(ErrorKind::Generic, msg.to_string())
}

/// Maps a missing optional host capability (`ctx.host.edits()`,
/// `.completions()`, `.ui()`, …, each `Option<&mut dyn Capability>`) to the
/// canonical "not supported by this host" error — see [`crate::host::unsupported`]
/// and the [`crate::host::EditorHost`] trait doc.
pub(crate) fn require_cap<'a, T: ?Sized>(
    cap: Option<&'a mut T>,
    name: &str,
) -> Result<&'a mut T, SteelErr> {
    cap.ok_or_else(|| generic_err(crate::host::unsupported(name)))
}

/// Strips a leading `%` from a registered Steel name for use in a gate
/// message. Several primitives (`%apply-text-edits!`, `%prompt!`, …) are
/// registered under an internal `%`-prefixed name but wrapped by a BOOTSTRAP
/// Scheme function under the friendly name a plugin author actually calls —
/// gate messages must name that friendly wrapper, not the internal primitive.
fn display_name(name: &str) -> &str {
    name.strip_prefix('%').unwrap_or(name)
}

/// Gate for builtins that touch live buffer/pane/editor state
/// (`PluginActivation` or `Command` only — never `Init` or `PluginLoad`,
/// where there is no meaningful focused buffer or live viewport yet).
pub(crate) fn require_cmd(ctx: &SteelCtx, name: &str) -> Result<(), SteelErr> {
    match ctx.mode() {
        EvalMode::Init | EvalMode::PluginLoad => {
            let name = display_name(name);
            steel::stop!(Generic => "{}: not available during init evaluation", name);
        }
        EvalMode::PluginActivation | EvalMode::Command => Ok(()),
    }
}

/// Gate for config/registration builtins (`set-option!`, `bind-key!`,
/// hook/LSP-server/language registration, …) — valid at init.scm top level or
/// inside any plugin body (eager load or lazy activation), but not from a
/// plain command body. Looser than [`require_cmd`]: see [`EvalMode`]'s doc
/// for why the two gates differ.
pub(crate) fn require_config(ctx: &SteelCtx, name: &str) -> Result<(), SteelErr> {
    match ctx.mode() {
        EvalMode::Command => {
            let name = display_name(name);
            steel::stop!(Generic =>
                "{}: only valid during init.scm or plugin load, not from a Steel command body",
                name);
        }
        EvalMode::Init | EvalMode::PluginLoad | EvalMode::PluginActivation => Ok(()),
    }
}

#[cfg(test)]
mod tests;
