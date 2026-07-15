//! `%stdout-gate!` — the Rust half of HUME's gated print builtins.
//!
//! steel-core's `displayln`/`display`/`print`/`println`/`newline`/`write`/
//! `write-string`/`write-char`/`simple-display`/`simple-displayln` all write
//! to the real process stdout by default. Calling any of them while HUME's
//! alt-screen TUI owns the terminal would corrupt the rendered frame.
//!
//! Why a plain top-level shadow misses plugin code: these ten names are
//! prelude exports, and steel-core prepends its prelude source to *every*
//! compiled unit, including each `(require "path.scm")` plugin file (a
//! separate compilation unit from HUME's top level). A top-level rebind —
//! `(define displayln …)` or `register_value`, in any form scoped to one
//! unit — only shadows the name in that unit; every plugin unit still
//! imports the original straight from the prelude.
//!
//! Fix: HUME appends its own gated redefinitions of all ten names to
//! steel-core's prelude string itself via `Engine::set_prelude_string` (see
//! `builtins/mod.rs::register_all`). Landing in every unit's prelude means
//! the shims shadow the imports inside every plugin module too.
//!
//! Two steel-core 0.8.2 wrinkles, both verified empirically:
//! - A single `compile_and_run_raw_program` call cannot both capture a
//!   name's current value and redefine that same name in the same unit
//!   (rejected at compile time). `register_all` therefore runs BOOTSTRAP
//!   (captures the originals) and `PRINT_GATE_SHIMS` (redefines the ten
//!   names) as two separate sequential calls — by the second, the names are
//!   ordinary already-bound globals, so redefining them is a plain rebind.
//! - A required module's compiled unit cannot contain a call site invoking a
//!   locally-shadowed prelude name with 2+ positional args (e.g. an
//!   explicit-port `(display obj port)`) when that name's shim declares a
//!   *mixed* fixed-plus-rest parameter list — reproduced independent of
//!   naming, even with `case-lambda`. `PRINT_GATE_SHIMS` therefore uses a
//!   *rest-only* parameter list for every shim, which dodges it; the
//!   implicit 0/1-arg form (the actual plugin use case) works everywhere.
//!   Residual gap: a plugin calling one of these names with an **explicit
//!   port argument from inside its own required-module body** still hits
//!   the limitation (no workaround short of patching steel-core; no real
//!   HUME code does this today) — the explicit-port form works everywhere
//!   else (top-level command bodies, `with-output-to-string`, etc).
//!
//! Explicit-port calls are gated too, not just forwarded — `port` can itself
//! be `(current-output-port)` (or the real stdout port via steel-core's own
//! error printer), which is exactly as unsafe as the implicit-port case.
//! [`stdout_gate`]'s Scheme-side caller, `%port-safe?`, checks the *supplied*
//! port's identity against the captured real stdout port, so a custom port
//! (string port, pipe) always passes through ungated. `write-string`/
//! `write-char` need the gate too even though steel-core's natives ignore
//! `(current-output-port)` in their 1-arg form and default straight to real
//! stdout — the shim explicitly threads it through so redirection
//! (`with-output-to-string`) works.
//!
//! This module provides only the gate check itself — [`stdout_gate`],
//! registered as `%stdout-gate!` — called by each Scheme shim before it
//! forwards to the captured original. See `PRINT_GATE_SHIMS` in
//! `builtins/mod.rs` for the shim definitions.

use steel::rerrs::SteelErr;
use steel::rvals::SteelVal;

use crate::SteelCtx;

/// Whether it is currently safe to write directly to the real process
/// stdout: init (before the alt-screen TUI is up) or an `#:inline-output`
/// command body (alt-screen temporarily left). See `SteelCtx::is_inline_output`.
fn stdout_is_safe(ctx: &SteelCtx) -> bool {
    ctx.is_inline_output || ctx.is_init
}

/// `(%stdout-gate!)` — called by each gated print shim (see
/// `PRINT_GATE_SHIMS` in `builtins/mod.rs`) immediately before it would write
/// to the real stdout. Returns `#f` (write must be suppressed) unless
/// [`stdout_is_safe`]. When safe via `is_inline_output` specifically (not
/// `is_init`, which prints pre-terminal with no bracket to open), lazily
/// enters the alt-screen bracket on this, the first real write of the
/// command body.
pub(crate) fn stdout_gate(ctx: &mut SteelCtx) -> Result<SteelVal, SteelErr> {
    if !stdout_is_safe(ctx) {
        return Ok(SteelVal::BoolV(false));
    }
    if ctx.is_inline_output {
        ctx.host
            .ensure_inline_output_screen()
            .map_err(|e| SteelErr::new(steel::rerrs::ErrorKind::Generic, format!("print: {e}")))?;
    }
    Ok(SteelVal::BoolV(true))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::null_host::{InlineOutputHost, RecordingInlineOutputHost};
    use crate::test_support::SteelCtxTestHarness;

    // ── stdout_is_safe: the actual gate logic ─────────────────────────────────
    //
    // These three cases pin the `||` semantics of `stdout_is_safe` — each one
    // distinguishes `||` from a wrong `&&`.
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

    // ── stdout_gate: behavior around the gate ──────────────────────────────────

    /// Gate closed: returns `#f`, no bracket entry.
    #[test]
    fn stdout_gate_returns_false_when_closed() {
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx(); // gate closed
        let result = stdout_gate(&mut ctx);
        assert_eq!(result.unwrap(), SteelVal::BoolV(false));
    }

    /// Gate open via `is_init` alone: returns `#t`, no bracket entry (there is
    /// no alt-screen to leave before the terminal exists).
    #[test]
    fn stdout_gate_returns_true_and_skips_ensure_when_open_via_is_init_only() {
        let mut host = RecordingInlineOutputHost::default();
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_init_with_host(&mut host); // is_init=true, is_inline_output=false
        let result = stdout_gate(&mut ctx);
        assert_eq!(result.unwrap(), SteelVal::BoolV(true));
        drop(ctx);
        assert_eq!(host.ensure_calls, 0);
    }

    /// Gate open via `is_inline_output`: returns `#t` and opens the bracket
    /// exactly once per call.
    #[test]
    fn stdout_gate_returns_true_and_calls_ensure_when_open_via_is_inline_output() {
        let mut host = RecordingInlineOutputHost::default();
        let mut h = SteelCtxTestHarness::new();
        let mut ctx = h.ctx_with_host(&mut host); // is_init=false, is_inline_output=true
        let result = stdout_gate(&mut ctx);
        assert_eq!(result.unwrap(), SteelVal::BoolV(true));
        drop(ctx);
        assert_eq!(host.ensure_calls, 1);
    }

    // ── End-to-end: gated print shims via the real ScriptingHost ───────────────

    /// Regression guard for the require-module bug documented in this
    /// module's doc comment: a plugin file loaded via `(require "path.scm")`
    /// is a separately-compiled module — exactly the shape of every real
    /// `#:inline-output` plugin command (core:lsp's `servers.scm`'s
    /// `lsp-servers`, core:plum's `grammars.scm`'s grammar-compile line).
    ///
    /// Defines a command inside a required module (mirroring how every real
    /// plugin command is defined) and dispatches it via `call_steel_cmd` — the
    /// same path `Editor::run_steel_command` uses — with a host reporting
    /// `is_inline_output_command() == true`. If the required module's
    /// `displayln` call resolved to steel-core's raw prelude version instead
    /// of the gated shim (the bug this test used to reproduce, before the
    /// prelude-injection fix), `ensure_calls` stays 0 despite the dispatch
    /// succeeding — the exact silent-bypass shape of the bug.
    #[test]
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
        let escaped_path = plugin_path.to_string_lossy().replace('\\', "\\\\");
        let src = format!(r#"(require "{escaped_path}")"#);
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

    /// The other four gated names (`display`, `newline`, `println` — `print`
    /// is exercised transitively via `println`) reach the gate from inside a
    /// required module too, not just `displayln`. Three implicit-port calls
    /// in one command body must open the bracket three times — once per
    /// gated call — since `RecordingInlineOutputHost` counts every
    /// `ensure_inline_output_screen` invocation unconditionally (unlike the
    /// real editor host, which only acts on the first).
    #[test]
    fn required_module_other_print_fns_reach_the_gate() {
        use crate::ScriptingHost;
        use crate::null_host::{NullHost, RecordingInlineOutputHost};

        let tmp = tempfile::tempdir().unwrap();
        let plugin_path = tmp.path().join("probe_multi.scm");
        std::fs::write(
            &plugin_path,
            r#"
                (define-command! "probe-print-multi"
                  "doc"
                  (lambda ()
                    (display "a")
                    (newline)
                    (println "b"))
                  #:inline-output #t)
            "#,
        )
        .unwrap();

        let mut host = ScriptingHost::new();
        let mut null_host = NullHost;
        let escaped_path = plugin_path.to_string_lossy().replace('\\', "\\\\");
        let src = format!(r#"(require "{escaped_path}")"#);
        host.eval_source(&src, &mut null_host)
            .expect("requiring the plugin file must not error");

        let mut recording_host = RecordingInlineOutputHost::default();
        host.call_steel_cmd(
            "probe-print-multi",
            None,
            vec![],
            hume_engine::pipeline::PaneId::default(),
            hume_engine::pipeline::BufferId::default(),
            &mut recording_host,
        )
        .expect("dispatching the required-module command must not error");

        assert_eq!(
            recording_host.ensure_calls, 3,
            "display, newline, and println must each reach the gate"
        );
    }

    /// The write-family names (`write`, `write-string`, `write-char`,
    /// `simple-display`, `simple-displayln`) reach the gate from inside a
    /// required module too — these were previously unshimmed, leaving every
    /// one of them an ungated path to the real stdout.
    #[test]
    fn required_module_write_family_reaches_the_gate() {
        use crate::ScriptingHost;
        use crate::null_host::{NullHost, RecordingInlineOutputHost};

        let tmp = tempfile::tempdir().unwrap();
        let plugin_path = tmp.path().join("probe_write.scm");
        std::fs::write(
            &plugin_path,
            r#"
                (define-command! "probe-write-multi"
                  "doc"
                  (lambda ()
                    (write 1)
                    (write-string "a")
                    (write-char #\c)
                    (simple-display "d")
                    (simple-displayln "e"))
                  #:inline-output #t)
            "#,
        )
        .unwrap();

        let mut host = ScriptingHost::new();
        let mut null_host = NullHost;
        let escaped_path = plugin_path.to_string_lossy().replace('\\', "\\\\");
        let src = format!(r#"(require "{escaped_path}")"#);
        host.eval_source(&src, &mut null_host)
            .expect("requiring the plugin file must not error");

        let mut recording_host = RecordingInlineOutputHost::default();
        host.call_steel_cmd(
            "probe-write-multi",
            None,
            vec![],
            hume_engine::pipeline::PaneId::default(),
            hume_engine::pipeline::BufferId::default(),
            &mut recording_host,
        )
        .expect("dispatching the required-module command must not error");

        assert_eq!(
            recording_host.ensure_calls, 5,
            "write, write-string, write-char, simple-display, and simple-displayln must each reach the gate"
        );
    }

    /// A `displayln` call at the top level (no `(require …)` involved) must
    /// also reach the gate — this is the load-bearing BOOTSTRAP-shim path,
    /// since steel never re-imports the print names into top-level programs.
    #[test]
    fn top_level_displayln_call_reaches_the_gate() {
        use crate::ScriptingHost;
        use crate::null_host::{NullHost, RecordingInlineOutputHost};

        let mut host = ScriptingHost::new();
        let mut null_host = NullHost;
        host.eval_source(
            r#"
                (define-command! "probe-print-top"
                  "doc"
                  (lambda ()
                    (displayln "hi-from-top-level"))
                  #:inline-output #t)
            "#,
            &mut null_host,
        )
        .expect("defining the top-level command must not error");

        let mut recording_host = RecordingInlineOutputHost::default();
        host.call_steel_cmd(
            "probe-print-top",
            None,
            vec![],
            hume_engine::pipeline::PaneId::default(),
            hume_engine::pipeline::BufferId::default(),
            &mut recording_host,
        )
        .expect("dispatching the top-level command must not error");

        assert_eq!(recording_host.ensure_calls, 1);
    }

    /// Writes to an explicit custom port (`with-output-to-string`) pass
    /// through untouched even when the gate is closed — `%stdout-safe?`'s
    /// `eq?` check against the real stdout port fails inside the
    /// parameterized dynamic extent, so the write is never suppressed.
    #[test]
    fn custom_port_write_bypasses_gate_when_closed() {
        use crate::ScriptingHost;
        use crate::null_host::NullHost;

        let mut host = ScriptingHost::new();
        let mut null_host = NullHost;
        host.eval_source(
            r#"
                (define-command! "probe-custom-port"
                  "doc"
                  (lambda ()
                    (log! 'info (with-output-to-string (lambda () (display "x") (newline))))))
            "#,
            &mut null_host,
        )
        .expect("defining the command must not error");

        // NullHost defaults is_inline_output_command() to false → gate closed.
        host.call_steel_cmd(
            "probe-custom-port",
            None,
            vec![],
            hume_engine::pipeline::PaneId::default(),
            hume_engine::pipeline::BufferId::default(),
            &mut null_host,
        )
        .expect("dispatching the command must not error");

        let messages = host.take_pending_messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].1, "x\n");
    }

    /// The explicit-port branch forwards straight to the original case-lambda
    /// rather than reimplementing arity checking — for a non-stdout port
    /// (`%port-safe?` is `#t` unconditionally), an extra positional argument
    /// still raises, exactly as it did before the gate existed.
    #[test]
    fn explicit_port_form_still_enforces_arity() {
        use crate::ScriptingHost;
        use crate::null_host::NullHost;

        let mut host = ScriptingHost::new();
        let mut null_host = NullHost;
        host.eval_source(
            r#"
                (define-command! "probe-bad-arity"
                  "doc"
                  (lambda ()
                    (call-with-output-string
                      (lambda (port) (display "a" port "extra")))))
            "#,
            &mut null_host,
        )
        .expect("defining the command must not error");

        let result = host.call_steel_cmd(
            "probe-bad-arity",
            None,
            vec![],
            hume_engine::pipeline::PaneId::default(),
            hume_engine::pipeline::BufferId::default(),
            &mut null_host,
        );
        assert!(result.is_err(), "extra positional arg must still error");
    }

    /// An explicit-port call where the supplied port genuinely IS the real
    /// stdout port (`(display obj (current-output-port))`, unparameterized)
    /// must still reach the gate — the bug this regression guards: the old
    /// shims forwarded any 2+-arg call unconditionally, so this exact call
    /// bypassed the gate entirely and wrote raw bytes onto the alt-screen.
    #[test]
    fn explicit_stdout_port_call_reaches_the_gate() {
        use crate::ScriptingHost;
        use crate::null_host::{NullHost, RecordingInlineOutputHost};

        let mut host = ScriptingHost::new();
        let mut null_host = NullHost;
        host.eval_source(
            r#"
                (define-command! "probe-explicit-stdout-port"
                  "doc"
                  (lambda ()
                    (display "x" (current-output-port)))
                  #:inline-output #t)
            "#,
            &mut null_host,
        )
        .expect("defining the command must not error");

        let mut recording_host = RecordingInlineOutputHost::default();
        host.call_steel_cmd(
            "probe-explicit-stdout-port",
            None,
            vec![],
            hume_engine::pipeline::PaneId::default(),
            hume_engine::pipeline::BufferId::default(),
            &mut recording_host,
        )
        .expect("dispatching the command must not error");

        assert_eq!(
            recording_host.ensure_calls, 1,
            "an explicit (current-output-port) argument that resolves to real \
             stdout must still open the bracket, not bypass the gate"
        );
    }

    /// `write-string`'s implicit 1-arg form must honor a redirected
    /// `(current-output-port)` (e.g. inside `with-output-to-string`) rather
    /// than falling through to steel-core's raw native, which ignores the
    /// parameter and always targets real stdout.
    #[test]
    fn write_string_implicit_form_honors_output_redirect() {
        use crate::ScriptingHost;
        use crate::null_host::NullHost;

        let mut host = ScriptingHost::new();
        let mut null_host = NullHost;
        host.eval_source(
            r#"
                (define-command! "probe-write-string-redirect"
                  "doc"
                  (lambda ()
                    (log! 'info (with-output-to-string (lambda () (write-string "x"))))))
            "#,
            &mut null_host,
        )
        .expect("defining the command must not error");

        // NullHost defaults is_inline_output_command() to false → gate closed —
        // pins that a captured, non-stdout port is never suppressed regardless.
        host.call_steel_cmd(
            "probe-write-string-redirect",
            None,
            vec![],
            hume_engine::pipeline::PaneId::default(),
            hume_engine::pipeline::BufferId::default(),
            &mut null_host,
        )
        .expect("dispatching the command must not error");

        let messages = host.take_pending_messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].1, "x");
    }
}
