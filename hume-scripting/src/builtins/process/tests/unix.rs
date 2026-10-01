//! Unix-only tests, gated once at the `mod unix;` declaration
//! in the parent.

use super::*;

#[test]
fn run_inline_output_returns_exit_code() {
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = run_inline_output(
        &mut ctx,
        "true".to_string(),
        list_val(&[]),
        SteelVal::BoolV(false),
        list_val(&[]),
    )
    .unwrap();
    assert_eq!(result, SteelVal::IntV(0));

    let result = run_inline_output(
        &mut ctx,
        "false".to_string(),
        list_val(&[]),
        SteelVal::BoolV(false),
        list_val(&[]),
    )
    .unwrap();
    assert_eq!(result, SteelVal::IntV(1));
}

#[test]
fn run_inline_output_honors_cwd_arg() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("marker.txt"), b"hi").unwrap();
    let mut h = SteelCtxTestHarness::new();
    let mut ctx = h.ctx();
    let result = run_inline_output(
        &mut ctx,
        "test".to_string(),
        list_val(&["-f", "marker.txt"]),
        SteelVal::StringV(tmp.path().to_string_lossy().into_owned().into()),
        list_val(&[]),
    )
    .unwrap();
    assert_eq!(
        result,
        SteelVal::IntV(0),
        "marker.txt must be found via cwd"
    );
}

#[test]
fn run_inline_output_scheme_wrapper_raises_on_nonzero_exit() {
    // End-to-end through the BOOTSTRAP `run-inline-output!` Scheme wrapper
    // (the #:cwd keyword sugar + raise-on-nonzero contract), not just the
    // raw `%run-inline-output!`.
    use crate::ScriptingHost;
    use crate::null_host::NullHost;

    let mut host = ScriptingHost::new();
    let mut null_host = NullHost;
    let ok_src = r#"(run-inline-output! "true" '())"#;
    host.eval_source(ok_src, &mut null_host)
        .expect("run-inline-output! success path must not raise");

    let mut host2 = ScriptingHost::new();
    let mut null_host2 = NullHost;
    let fail_src = r#"
        (with-handler
          (lambda (err)
            (if (string-contains? (to-string err) "false")
                (begin)
                (error (string-append "error did not name cmd: " (to-string err)))))
          (begin
            (run-inline-output! "false" '())
            (error "expected run-inline-output! to raise on nonzero exit")))
    "#;
    host2
        .eval_source(fail_src, &mut null_host2)
        .expect("run-inline-output! failure-path assertion failed");
}

#[test]
fn run_inline_output_scheme_wrapper_passes_env() {
    use crate::ScriptingHost;
    use crate::null_host::NullHost;

    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("out.txt");
    let src = format!(
        r#"(run-inline-output! "sh"
             (list "-c" "printf %s \"$HUME_ENV_PROBE\" > {}")
             #:env '(("HUME_ENV_PROBE" . "probe-value")))"#,
        out.display()
    );
    let mut host = ScriptingHost::new();
    let mut null_host = NullHost;
    host.eval_source(&src, &mut null_host)
        .expect("run-inline-output! with #:env must not raise");
    assert_eq!(fs::read_to_string(out).unwrap(), "probe-value");
}
