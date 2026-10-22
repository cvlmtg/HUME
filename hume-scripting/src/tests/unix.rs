//! Unix-only steel-stdlib-availability tests, gated once at the
//! `mod unix;` declaration in the parent.

use super::*;
use hume_platform::dirs::Dirs;

/// Pins a real gotcha in Steel's own `steel/process` stdlib: `child-stderr`
/// (and by extension `child-stdin`/`child-stdout`) must be captured
/// *before* calling `wait`: calling it after returns `#f` even though the
/// stream was piped. Also pins the stdin-close-for-EOF pattern needed
/// since stdin is not inherited by default. No in-tree Scheme calls
/// `spawn-process` directly any more (`core:stdlib`'s `stdlib/run!` is now
/// `run-capture!`, a native builtin closing the exact deadlock this shape
/// invites; see its own doc), but the full-trust plugin model leaves
/// `spawn-process` reachable from any user plugin, so the gotcha stays live
/// and worth pinning.
#[test]
fn child_stderr_must_be_captured_before_wait() {
    let mut host = ScriptingHost::new(&Dirs::none());
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
        .expect("spawn-process shape probe failed");
}

/// A native builtin's error re-raised from an inner `with-handler` reaches
/// the outer one, the shape of a cleanup-and-reraise wrapper called from a
/// tolerant caller.
#[test]
fn native_error_reraised_from_a_handler_reaches_the_enclosing_handler() {
    let mut host = ScriptingHost::new(&Dirs::none());
    let mut null_host = NullHost;
    let src = r#"
        (define (inner-fetch)
          (with-handler
            (lambda (err) (raise-error err))
            (run-inline-output! "false" '())))

        (define (tolerant-outer)
          (with-handler (lambda (err) (error-object-message err)) (inner-fetch)))

        (define message (tolerant-outer))
        (if (string-contains? message "run-inline-output!: false failed")
            (begin)
            (error (string-append "outer handler saw: " message)))
    "#;
    host.eval_source(src, &mut null_host)
        .expect("the outer handler must receive the re-raised native error");
}

#[test]
fn uncaught_native_error_propagates_one_hop_to_outer_tolerant_handler() {
    // Shape: the native-builtin-raising call (run-inline-output!) is NOT
    // wrapped by an inner with-handler at all; it propagates in one hop
    // straight to the outer tolerant handler.
    let mut host = ScriptingHost::new(&Dirs::none());
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

/// `dynamic-wind`'s after thunk runs when its body raises a native error
/// that an outer `with-handler` catches.
#[test]
fn dynamic_wind_cleanup_runs_when_an_outer_handler_catches_the_bodys_error() {
    let mut host = ScriptingHost::new(&Dirs::none());
    let mut null_host = NullHost;
    let src = r#"
        (define cleanup-ran #f)
        (define (inner-fetch)
          (dynamic-wind
            (lambda () #t)
            (lambda () (run-inline-output! "false" '()))
            (lambda () (set! cleanup-ran #t))))

        (define (tolerant-outer)
          (with-handler (lambda (err) #f) (inner-fetch)))

        (tolerant-outer)
        (if cleanup-ran (begin) (error "cleanup did not run"))
    "#;
    host.eval_source(src, &mut null_host)
        .expect("dynamic-wind's after thunk must run before the outer handler returns");
}
