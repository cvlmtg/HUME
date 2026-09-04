use super::*;
use crate::null_host::RecordingInlineOutputHost;
use crate::test_support::SteelCtxTestHarness;

// ── stdout_is_safe: the actual gate logic ─────────────────────────────────
//
// These three cases pin the `||` semantics of `stdout_is_safe` — each one
// distinguishes `||` from a wrong `&&`.
//
// Fail oracle: change `stdout_is_safe` to `inline && ctx.session ==
// EvalSession::Init` → `neither_flag_set_is_unsafe` still passes, but the
// other two flip to `false` and fail.

#[test]
fn neither_flag_set_is_unsafe() {
    let mut h = SteelCtxTestHarness::new();
    let ctx = h.ctx(); // NullHost: EvalSession::Runtime, output() is None
    assert!(!stdout_is_safe(&ctx, false));
}

#[test]
fn init_session_alone_is_safe() {
    let mut h = SteelCtxTestHarness::new();
    let ctx = h.ctx_init(); // EvalSession::Init, output() is None
    assert!(stdout_is_safe(&ctx, false));
}

#[test]
fn is_inline_output_alone_is_safe() {
    let mut host = RecordingInlineOutputHost::default();
    let mut h = SteelCtxTestHarness::new();
    let ctx = h.ctx_with_host(&mut host); // EvalSession::Runtime, host reports inline_output=true
    assert!(stdout_is_safe(&ctx, true));
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

/// Gate open via the init session alone: returns `#t`, no bracket entry
/// (there is no alt-screen to leave before the terminal exists).
///
/// `inline_output: false` isolates this from the *other* safety reason —
/// without it, `RecordingInlineOutputHost`'s own default (`true`) would make
/// this pass for the wrong reason, since `stdout_is_safe` reads the host
/// live regardless of session.
#[test]
fn stdout_gate_returns_true_and_skips_ensure_when_open_via_init_session_only() {
    let mut host = RecordingInlineOutputHost::default();
    host.inline_output = false;
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_init_with_host(&mut host); // EvalSession::Init, host reports inline_output=false
    let result = stdout_gate(&mut ctx);
    assert_eq!(result.unwrap(), SteelVal::BoolV(true));
    drop(ctx);
    assert_eq!(host.ensure_calls, 0);
}

/// Gate open via the host's inline-output flag: returns `#t` and opens the
/// bracket exactly once per call.
#[test]
fn stdout_gate_returns_true_and_calls_ensure_when_open_via_inline_output_command() {
    let mut host = RecordingInlineOutputHost::default();
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx_with_host(&mut host); // EvalSession::Runtime, host reports inline_output=true
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
/// of the gated shim, `ensure_calls` stays 0 despite the dispatch
/// succeeding — the exact silent-bypass shape this test guards against.
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
/// required module too — an unshimmed member of this family would be an
/// ungated path to the real stdout.
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
