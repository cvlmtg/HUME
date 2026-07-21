//! Unix-only steel-stdlib-availability tests, gated once at the
//! `mod unix;` declaration in the parent.

use super::*;

/// Pins a real gotcha `plum/run!` (Phase 1 helper) depends on: `child-stderr`
/// (and by extension `child-stdin`/`child-stdout`) must be captured
/// *before* calling `wait` — calling it after returns `#f` even though the
/// stream was piped. Also pins the stdin-close-for-EOF pattern needed
/// since stdin is not inherited by default.
#[test]
fn child_stderr_must_be_captured_before_wait() {
    let mut host = ScriptingHost::new();
    let mut null_host = NullHost;
    // Failure path: stdout+stderr+stdin all piped, stdin closed
    // immediately, wait for exit, read stderr on nonzero.
    let src = r#"
        (define builder
          (with-stdin-piped (with-stderr-piped (with-stdout-piped (command "sh" (list "-c" "echo err-marker 1>&2; exit 3"))))))
        (define spawned (spawn-process builder))
        (if (Ok? spawned)
            (let* ([child (Ok->value spawned)]
                   [stderr-port (child-stderr child)])
              (close-output-port (child-stdin child))
              (let ([wait-result (wait child)])
                (if (Ok? wait-result)
                    (let ([code (Ok->value wait-result)])
                      (if (= code 3)
                          (let ([stderr (read-port-to-string stderr-port)])
                            (if (string-contains? stderr "err-marker")
                                (begin)
                                (error (string-append "stderr missing marker: " stderr))))
                          (error (string-append "unexpected exit code: " (to-string code)))))
                    (error (to-string (Err->value wait-result))))))
            (error (to-string (Err->value spawned))))
    "#;
    host.eval_source(src, &mut null_host)
        .expect("plum/run! shape probe failed");
}

/// **Known steel-core 0.8.2 limitation, not a HUME bug**: re-raising a
/// native-builtin error (via `raise-error`) from an inner `with-handler`,
/// caught by an *outer* `with-handler`, corrupts the VM's continuation
/// stack and panics "Failed to find an open continuation on the stack".
/// Also reachable via `grammars.scm`'s `plum/resolve-query` (see
/// `plum/fetch-raw-query`'s doc comment for the fix: never wrap the
/// raising call in an inner catch-and-reraise). `#[should_panic]`
/// regression pin — if a steel-core upgrade fixes this, revisit the
/// `grammars.scm` workaround.
#[test]
#[should_panic(expected = "Failed to find an open continuation on the stack")]
fn known_limitation_reraise_via_raise_error_inside_outer_tolerant_handler_corrupts_vm_stack() {
    let mut host = ScriptingHost::new();
    let mut null_host = NullHost;
    // Exact shape of plum/fetch-raw-query (inner: catch, cleanup, re-raise
    // via raise-error) nested inside plum/resolve-query's tolerant outer
    // with-handler.
    let src = r#"
        (define (inner-fetch)
          (with-handler
            (lambda (err) (raise-error err))
            (run-inline-output! "false" '())))

        (define (tolerant-outer)
          (with-handler (lambda (err) #f) (inner-fetch)))

        (tolerant-outer)
    "#;
    host.eval_source(src, &mut null_host)
        .expect("raise-error re-raise inside outer tolerant handler failed");
}

#[test]
fn uncaught_native_error_propagates_one_hop_to_outer_tolerant_handler() {
    // Fix shape: the native-builtin-raising call (run-inline-output!) is
    // NOT wrapped by an inner with-handler at all — it propagates in one
    // hop straight to the outer tolerant handler, exactly like the
    // original (pre-migration) curl-fetch call site.
    let mut host = ScriptingHost::new();
    let mut null_host = NullHost;
    let src = r#"
        (define (inner-fetch)
          (run-inline-output! "false" '()))

        (define (tolerant-outer)
          (with-handler (lambda (err) #f) (inner-fetch)))

        (tolerant-outer)
    "#;
    host.eval_source(src, &mut null_host)
        .expect("uncaught native error one-hop propagation to outer handler failed");
}

/// **Second known steel-core 0.8.2 limitation**: `dynamic-wind`'s
/// `after` thunk is not guaranteed to run when its body raises through an
/// outer `with-handler` — reproduces the panic-pinning test's failure,
/// wrapped in `dynamic-wind` instead of catch-and-reraise. This would
/// otherwise be a safe way to guarantee `declare-plugin`'s manifest
/// cleanup (`%finish-manifest-declare!`) runs without an inner handler,
/// but `cleanup-ran` never fires — confirms the decision (see
/// `project_steel_raii_vs_dynamicwind.md`) to keep cleanup-on-unwind in
/// Rust (explicit push/pop), never Steel `dynamic-wind`. Pinned like the
/// test above: a steel-core fix flips `cleanup-ran` to `#t` and this
/// starts failing — revisit then.
#[test]
fn known_limitation_dynamic_wind_cleanup_does_not_run_across_an_outer_handlers_unwind() {
    let mut host = ScriptingHost::new();
    let mut null_host = NullHost;
    let src = r#"
        (define cleanup-ran #f)
        (define (inner-fetch)
          (dynamic-wind
            (lambda () (void))
            (lambda () (run-inline-output! "false" '()))
            (lambda () (set! cleanup-ran #t))))

        (define (tolerant-outer)
          (with-handler (lambda (err) #f) (inner-fetch)))

        (tolerant-outer)
        (if cleanup-ran (begin) (error "cleanup did not run"))
    "#;
    let result = host.eval_source(src, &mut null_host);
    let err = result.expect_err(
        "dynamic-wind's cleanup thunk unexpectedly ran across the outer handler's unwind — \
         if steel-core fixed this, declare-plugin's manifest branch could use dynamic-wind \
         instead of catch-and-reraise to avoid the panic pinned above",
    );
    assert!(
        err.contains("cleanup did not run"),
        "expected the cleanup-did-not-run assertion to fire, got a different error: {err}"
    );
}

