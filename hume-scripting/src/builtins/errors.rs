//! One error idiom for builtins: `steel::stop!` for early returns,
//! [`generic_err`] for `map_err`/closure positions where an early return
//! doesn't fit. [`named`] puts the builtin's name in front of every error,
//! so no message names its own builtin. Plus the two `EvalMode` gate checks
//! every config/command builtin opens with.
//!
//! No other module constructs a `SteelErr` directly, enforced by
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
/// one "not supported by this host" error. See the
/// [`crate::host::EditorHost`] trait doc.
pub(crate) fn require_cap<T: ?Sized>(cap: Option<&mut T>) -> Result<&mut T, SteelErr> {
    cap.ok_or_else(|| generic_err("not supported by this host"))
}

/// Runs `body`, the builtin a plugin calls as `builtin`, its error starting
/// with that name: the one place a builtin's name enters its error. The
/// `builtins!` table runs every builtin through it.
///
/// # Panics
/// In a debug build, when the error already starts with `builtin`: its
/// message named its own builtin.
pub(crate) fn named<T>(
    builtin: &str,
    body: impl FnOnce() -> Result<T, SteelErr>,
) -> Result<T, SteelErr> {
    body().map_err(|mut e| {
        debug_assert!(
            !e.to_string()
                .starts_with(&format!("Error: {:?}: {builtin}: ", e.kind())),
            "{builtin} named itself in its error: {e}"
        );
        e.prepend_message(": ");
        e.prepend_message(builtin);
        e
    })
}

/// Gate for builtins that touch live buffer/pane/editor state
/// (`PluginActivation` or `Command` only, never `Init` or `PluginLoad`,
/// where there is no meaningful focused buffer or live viewport yet).
pub(crate) fn require_cmd(ctx: &SteelCtx) -> Result<(), SteelErr> {
    match ctx.mode() {
        EvalMode::Init | EvalMode::PluginLoad => {
            steel::stop!(Generic => "not available during init evaluation");
        }
        EvalMode::PluginActivation | EvalMode::Command => Ok(()),
    }
}

/// Gate for config/registration builtins (`set-option!`, a global `bind-key!`,
/// hook/LSP-server/language registration, …): valid at init.scm top level or
/// inside any plugin body (eager load or lazy activation), but not from a
/// plain command body. Looser than [`require_cmd`]: see [`EvalMode`]'s doc
/// for why the two gates differ.
pub(crate) fn require_config(ctx: &SteelCtx) -> Result<(), SteelErr> {
    match ctx.mode() {
        EvalMode::Command => {
            steel::stop!(Generic =>
                "only valid during init.scm or plugin load, not from a Steel command body");
        }
        EvalMode::Init | EvalMode::PluginLoad | EvalMode::PluginActivation => Ok(()),
    }
}

#[cfg(test)]
mod tests;
