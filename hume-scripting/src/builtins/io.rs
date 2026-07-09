//! `displayln` — a gated, TUI-safe shadow of steel-core's stdout builtin.
//!
//! steel-core's `kernel.scm` binds `displayln` to a raw `print!` on the real
//! process stdout (see `IoFunctions::displayln` in steel-core's
//! `primitives/io.rs`). Calling that while HUME's alt-screen TUI owns the
//! terminal would corrupt the rendered frame. `register_all` (see
//! `builtins/mod.rs`) re-registers `%displayln!` after `Engine::new()`, and the
//! BOOTSTRAP shim rebinds the Scheme-visible name `displayln` to call it —
//! shadowing the kernel's version everywhere in HUME's runtime.

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
    use crate::null_host::InlineOutputHost;
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
}
