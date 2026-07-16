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
mod tests {
    use super::*;
    use crate::attribution::PluginId;
    use crate::test_support::SteelCtxTestHarness;

    /// `require_cmd` rejects `Init` and `PluginLoad`, allows the other two.
    ///
    /// Independent oracle: expected pass/fail per state comes from
    /// `EvalMode`'s doc table, not from `require_cmd`'s own logic.
    #[test]
    fn require_cmd_gates_by_mode() {
        let mut h = SteelCtxTestHarness::new();
        assert!(require_cmd(&h.ctx_init(), "x").is_err(), "Init must reject");
        assert!(require_cmd(&h.ctx(), "x").is_ok(), "Command must pass");

        h.plugin_stack
            .push(PluginId::parse("core:test-plugin").unwrap());
        assert!(
            require_cmd(&h.ctx_init(), "x").is_err(),
            "PluginLoad must reject"
        );
        assert!(
            require_cmd(&h.ctx_activation(), "x").is_ok(),
            "PluginActivation must pass"
        );
    }

    /// `require_config` rejects only `Command`, allows the other three.
    #[test]
    fn require_config_gates_by_mode() {
        let mut h = SteelCtxTestHarness::new();
        assert!(require_config(&h.ctx_init(), "x").is_ok(), "Init must pass");
        assert!(
            require_config(&h.ctx(), "x").is_err(),
            "Command must reject"
        );

        h.plugin_stack
            .push(PluginId::parse("core:test-plugin").unwrap());
        assert!(
            require_config(&h.ctx_init(), "x").is_ok(),
            "PluginLoad must pass"
        );
        assert!(
            require_config(&h.ctx_activation(), "x").is_ok(),
            "PluginActivation must pass"
        );
    }

    /// `require_cmd`'s error message names the builtin and mentions "init".
    ///
    /// Fail oracle: drop the `name` interpolation → message no longer
    /// identifies which builtin rejected the call.
    #[test]
    fn require_cmd_error_names_builtin() {
        let mut h = SteelCtxTestHarness::new();
        let err = require_cmd(&h.ctx_init(), "close-buffer!").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("close-buffer!"), "got: {msg}");
        assert!(msg.contains("not available during init"), "got: {msg}");
    }

    /// `require_config`'s error message names the builtin and mentions
    /// "command body".
    #[test]
    fn require_config_error_names_builtin() {
        let mut h = SteelCtxTestHarness::new();
        let err = require_config(&h.ctx(), "set-option!").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("set-option!"), "got: {msg}");
        assert!(msg.contains("command body"), "got: {msg}");
    }

    /// A `%`-prefixed registration name (a Rust primitive wrapped by a
    /// BOOTSTRAP Scheme function) surfaces in the gate message WITHOUT the
    /// `%` — the message must name the wrapper a plugin author actually
    /// calls, not the internal primitive.
    ///
    /// Fail oracle: pass `name` straight through without stripping → the
    /// message contains "%apply-text-edits!" instead of "apply-text-edits!".
    #[test]
    fn gate_strips_leading_percent_from_registered_name() {
        let mut h = SteelCtxTestHarness::new();
        let err = require_cmd(&h.ctx_init(), "%apply-text-edits!").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("apply-text-edits!"), "got: {msg}");
        assert!(!msg.contains("%apply-text-edits!"), "got: {msg}");
    }

    /// `generic_err` preserves the source message verbatim and constructs a
    /// `Generic`-kind error (surfaced only via `Display`, not asserted
    /// elsewhere — no test in this crate checks `ErrorKind`).
    #[test]
    fn generic_err_preserves_message() {
        let err = generic_err("buffer-path: no such buffer");
        assert!(err.to_string().contains("no such buffer"));
    }
}
