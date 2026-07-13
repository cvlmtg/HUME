//! `displayln` — a gated, TUI-safe shadow of steel-core's stdout builtin.
//!
//! steel-core's own prelude binds `displayln` to a raw `print!` on the real
//! process stdout. Calling that while HUME's alt-screen TUI owns the terminal
//! would corrupt the rendered frame. `register_all` (see `builtins/mod.rs`)
//! registers `%displayln!` after `Engine::new()`, and the BOOTSTRAP shim
//! rebinds the Scheme-visible name `displayln` to call it, gated on
//! `SteelCtx::is_inline_output`/`is_init` via [`stdout_is_safe`].
//!
//! **KNOWN GAP**: this shadow does not reach code compiled via `(require
//! "path.scm")` — i.e. every real HUME plugin (PLUM, per the namespace-
//! isolation decision in `docs/ROADMAP.md`). A `displayln` call from inside a
//! required module's command body resolves to steel-core's raw, ungated
//! kernel version instead of this gate — confirmed empirically, root cause
//! not yet understood (two fix attempts, a reserved-slot `#%prim.displayln`
//! registration and a direct bare-name registration, both failed to close
//! the gap). See the KNOWN GAP note on `%displayln!`'s registration in
//! `builtins/mod.rs`, and the `#[ignore]`d
//! `required_module_displayln_call_reaches_the_gate` test below, which is the
//! live reproduction.
//!
//! When the gate is open via `is_inline_output`, a call is also meant to be
//! the trigger that lazily opens the alt-screen bracket — see
//! `EditorHost::ensure_inline_output_screen` — but this only happens for code
//! that actually reaches [`displayln`] below, which the known gap above
//! excludes for required-module callers.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;

/// Whether it is currently safe to write directly to the real process
/// stdout: init (before the alt-screen TUI is up) or an `#:inline-output`
/// command body (alt-screen temporarily left). See `SteelCtx::is_inline_output`.
fn stdout_is_safe(ctx: &SteelCtx) -> bool {
    ctx.is_inline_output || ctx.is_init
}

/// `(%displayln! args)` — `args` is the Scheme rest-list collected by the
/// `(define (displayln . args) (%displayln! args))` shim in BOOTSTRAP.
///
/// No-ops (returns `#<void>` without touching stdout) unless
/// [`stdout_is_safe`]. When safe, forwards `args` verbatim to steel-core's
/// own `displayln` implementation rather than reimplementing it.
pub(crate) fn displayln(ctx: &mut SteelCtx, args: SteelVal) -> Result<SteelVal, SteelErr> {
    if !stdout_is_safe(ctx) {
        return Ok(SteelVal::Void);
    }
    let SteelVal::ListV(list) = args else {
        steel::stop!(TypeMismatch => "displayln: expected an arg list, got {:?}", args);
    };
    // Gate on `is_inline_output` specifically, not `stdout_is_safe`'s `||`:
    // `is_init` prints run pre-terminal, where there is no alt-screen bracket
    // to open at all.
    if ctx.is_inline_output {
        ctx.host.ensure_inline_output_screen().map_err(|e| {
            SteelErr::new(steel::rerrs::ErrorKind::Generic, format!("displayln: {e}"))
        })?;
    }
    let SteelVal::FuncV(core_displayln) = steel::primitives::IoFunctions::displayln() else {
        unreachable!("IoFunctions::displayln always returns FuncV")
    };
    let items: Vec<SteelVal> = list.into_iter().collect();
    core_displayln(&items)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::null_host::{InlineOutputHost, RecordingInlineOutputHost};
    use crate::test_support::SteelCtxTestHarness;

    fn list_of(items: Vec<SteelVal>) -> SteelVal {
        use steel::rvals::IntoSteelVal;
        items.into_steelval().expect("list conversion")
    }

    // ── stdout_is_safe: the actual gate logic ─────────────────────────────────
    //
    // `displayln` itself can't be asserted on directly for the print-vs-no-op
    // split (both branches return `#<void>`, so an assertion on the return
    // value alone would pass even if the gate were removed entirely). These
    // three cases pin the `||` semantics of `stdout_is_safe` instead — each
    // one distinguishes `||` from a wrong `&&`.
    //
    // Fail oracle: change `stdout_is_safe` to `ctx.is_inline_output &&
    // ctx.is_init` → `neither_flag_set_is_unsafe` still passes, but the other
    // two flip to `false` and fail.

    #[test]
    fn neither_flag_set_is_unsafe() {
        let mut h = SteelCtxTestHarness::new();
        let ctx = h.ctx(); // NullHost: is_init=false, is_inline_output=false
        assert!(!stdout_is_safe(&ctx));
    }

    #[test]
    fn is_init_alone_is_safe() {
        let mut h = SteelCtxTestHarness::new();
        let ctx = h.ctx_init(); // is_init=true, is_inline_output=false
        assert!(stdout_is_safe(&ctx));
    }

    #[test]
    fn is_inline_output_alone_is_safe() {
        let mut host = InlineOutputHost;
        let mut h = SteelCtxTestHarness::new();
        let ctx = h.ctx_with_host(&mut host); // is_init=false, is_inline_output=true
        assert!(stdout_is_safe(&ctx));
    }

    // ── displayln: behavior around the gate ────────────────────────────────────

    /// Gate closed: no-ops without inspecting `args` at all — a malformed
    /// `args` value must not surface as an error when the gate is shut.
    #[test]
    fn displayln_noops_and_skips_arg_validation_when_gate_closed() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx(); // gate closed
        let result = displayln(&mut ctx, SteelVal::StringV("not-a-list".into()));
        assert_eq!(result.unwrap(), SteelVal::Void);
    }

    /// A non-list `args` value is a type error once the gate is open — even
    /// though the gate lets it through, malformed input from the BOOTSTRAP
    /// shim (or a future caller) must surface, not vanish.
    #[test]
    fn displayln_rejects_non_list_args_when_gate_open() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init(); // gate open
        let result = displayln(&mut ctx, SteelVal::StringV("not-a-list".into()));
        assert!(result.is_err());
    }

    /// Valid list args through the open gate forward successfully.
    #[test]
    fn displayln_forwards_valid_list_when_gate_open() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init(); // gate open
        let result = displayln(&mut ctx, list_of(vec![SteelVal::StringV("hi".into())]));
        assert_eq!(result.unwrap(), SteelVal::Void);
    }

    // ── ensure_inline_output_screen: lazy bracket entry ─────────────────────

    /// A real `#:inline-output` print opens the bracket exactly once.
    #[test]
    fn displayln_calls_ensure_when_open_via_is_inline_output() {
        let mut host = RecordingInlineOutputHost::default();
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_with_host(&mut host); // is_init=false, is_inline_output=true
        let result = displayln(&mut ctx, list_of(vec![SteelVal::StringV("hi".into())]));
        assert!(result.is_ok());
        drop(ctx);
        assert_eq!(host.ensure_calls, 1);
    }

    /// An init-time print (`is_init` alone) never opens the bracket — there is
    /// no alt-screen to leave before the terminal exists.
    #[test]
    fn displayln_does_not_call_ensure_when_open_via_is_init_only() {
        let mut host = RecordingInlineOutputHost::default();
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init_with_host(&mut host); // is_init=true, is_inline_output=false
        let result = displayln(&mut ctx, list_of(vec![SteelVal::StringV("hi".into())]));
        assert!(result.is_ok());
        drop(ctx);
        assert_eq!(host.ensure_calls, 0);
    }

    /// End-to-end regression for the require-module bug documented in this
    /// module's doc comment: a plugin file loaded via `(require "path.scm")`
    /// is a separately-compiled module — exactly the shape of every real
    /// `#:inline-output` PLUM command (`servers.scm`'s `lsp-servers`,
    /// `grammars.scm`'s grammar-compile line).
    ///
    /// Defines a command inside a required module (mirroring how every real
    /// plugin command is defined) and dispatches it via `call_steel_cmd` — the
    /// same path `Editor::run_steel_command` uses — with a host reporting
    /// `is_inline_output_command() == true`. If the required module's
    /// `displayln` call resolved to steel-core's raw kernel version instead of
    /// this module's gated `displayln`, `ensure_calls` stays 0 despite the
    /// dispatch succeeding — the exact silent-bypass shape of the bug.
    ///
    /// KNOWN FAILING — kept `#[ignore]`d as a live reproduction. Un-ignore
    /// once the real fix lands; this assertion is the actual regression
    /// guard for whatever that fix turns out to be.
    #[test]
    #[ignore = "known gap: required-module displayln bypasses the gate — see KNOWN GAP note in builtins/mod.rs::register_all"]
    fn required_module_displayln_call_reaches_the_gate() {
        use crate::ScriptingHost;
        use crate::null_host::{NullHost, RecordingInlineOutputHost};

        let tmp = tempfile::tempdir().unwrap();
        let plugin_path = tmp.path().join("probe.scm");
        std::fs::write(
            &plugin_path,
            r#"
                (define-command! "probe-print"
                  "doc"
                  (lambda ()
                    (displayln "hi-from-required-module"))
                  #:inline-output #t)
            "#,
        )
        .unwrap();

        let mut host = ScriptingHost::new();
        let mut null_host = NullHost;
        let src = format!(r#"(require "{}")"#, plugin_path.to_string_lossy());
        host.eval_source(&src, &mut null_host)
            .expect("requiring the plugin file must not error");

        let mut recording_host = RecordingInlineOutputHost::default();
        host.call_steel_cmd(
            "probe-print",
            None,
            vec![],
            hume_engine::pipeline::PaneId::default(),
            hume_engine::pipeline::BufferId::default(),
            &mut recording_host,
        )
        .expect("dispatching the required-module command must not error");

        assert_eq!(
            recording_host.ensure_calls, 1,
            "required-module displayln call must reach the gate"
        );
    }
}
