;;; core:plum/lib.scm

(provide plum/valid-dir-entry? plum/batch-run plum/find
         plum/run! plum/list-dir plum/read-file plum/write-file
         plum/delete-dir plum/delete-file)

;; ── Directory entry filter ────────────────────────────────────────────────────

;;; Return #t if `name` is a valid, traversable directory entry (not "." or "..").
(define (plum/valid-dir-entry? name)
  (and (not (equal? name "."))
       (not (equal? name ".."))))

;; ── List search ───────────────────────────────────────────────────────────────

;;; First element of `lst` satisfying `pred?`, or `#f`.
(define (plum/find pred? lst)
  (cond ((null? lst) #f)
        ((pred? (car lst)) (car lst))
        (else (plum/find pred? (cdr lst)))))

;; ── Process spawning ──────────────────────────────────────────────────────────
;;
;; Full-trust plugin model (see docs/ROADMAP.md): PLUM spawns processes via
;; Steel's own `steel/process` stdlib, not a hardened Rust builtin. The one
;; exception is `#:inline-output` commands, which use the separate
;; `run-inline-output!` builtin for process-group (Ctrl+C) safety — see its
;; doc comment. `plum/run!` below is for everywhere else (plain `git-clone`,
;; `git-pull`) — commands that run with the TUI's terminal raw mode still on,
;; where Steel's own `spawn-process` is safe to use directly.

;;; Spawn `cmd` with `args` (a list of strings), capturing stdout+stderr;
;;; blocks until exit. On a nonzero exit, spawn failure, or wait failure,
;;; raises an error naming `cmd` and including stderr. `#:cwd`, if given,
;;; sets the child's working directory.
;;;
;;; stdin is piped and closed immediately after spawn — never left open
;;; waiting for input, and never inherited from HUME's own terminal (which
;;; would let the child's reads race the editor's own key reads). This
;;; mirrors the non-inherited-stdin contract of the `Command::output()`-based
;;; builtin this replaces.
;;;
;;; Gotcha (pinned by a permanent test in hume-scripting): `child-stderr`
;;; must be grabbed before `wait` is called, or it returns `#f` even though
;;; the stream was piped.
(define (plum/run! cmd args #:cwd [dir #f])
  (let* ([base (with-stdin-piped (with-stderr-piped (with-stdout-piped (command cmd args))))]
         [builder (if dir (with-current-dir base dir) base)]
         [spawned (spawn-process builder)])
    (if (Ok? spawned)
        (let* ([child (Ok->value spawned)]
               [stderr-port (child-stderr child)])
          (close-output-port (child-stdin child))
          (let ([wait-result (wait child)])
            (if (Ok? wait-result)
                (let ([code (Ok->value wait-result)])
                  (unless (= code 0)
                    (let ([stderr (trim (read-port-to-string stderr-port))])
                      (error (string-append cmd ": failed (exit " (number->string code) "): " stderr)))))
                (error (string-append cmd ": wait failed: " (to-string (Err->value wait-result)))))))
        (error (string-append cmd ": cannot spawn: " (to-string (Err->value spawned)))))))

;; ── Filesystem helpers ────────────────────────────────────────────────────────
;;
;; Thin wrappers over Steel's `steel/filesystem`/`steel/ports` matching the
;; contracts of the removed HUME builtins of similar name, so call sites
;; elsewhere in PLUM don't need to change beyond the require.

;;; Sorted list of basenames in `dir` (mirrors the removed `list-dir`
;;; builtin's contract — Steel's own `read-dir` returns full paths instead).
(define (plum/list-dir dir)
  (sort (map file-name (read-dir dir)) string<?))

;;; Full contents of the file at `path`, as a string.
(define (plum/read-file path)
  (let ([port (open-input-file path)])
    (let ([content (read-port-to-string port)])
      (close-input-port port)
      content)))

;;; Write `content` to `path`, creating or truncating it.
(define (plum/write-file path content)
  (let ([port (open-output-file path)])
    (write-string content port)
    (close-output-port port)))

;;; Recursively delete `dir`. Idempotent — a missing directory is not an
;;; error — matching the removed `delete-dir` builtin's contract; Steel's own
;;; `delete-directory!` raises on a missing path, and several call sites
;;; (e.g. clearing a stale source tree before a first-time clone) rely on
;;; being able to call this whether or not anything is there yet.
(define (plum/delete-dir dir)
  (when (path-exists? dir)
    (delete-directory! dir)))

;;; Delete the file at `path`. Idempotent — a missing file is not an error —
;;; matching the removed `delete-file` builtin's contract; Steel's own
;;; `delete-file!` raises on a missing path, and cleanup-on-failure call
;;; sites (e.g. removing a partial download) must tolerate the file never
;;; having been created.
(define (plum/delete-file path)
  (when (path-exists? path)
    (delete-file! path)))

;; ── Batch runner ──────────────────────────────────────────────────────────────

;;; Run `thunk` on each name in `names`, collecting errors rather than
;;; aborting.  Logs per-item progress and a summary at the end.
;;; Returns the count of successful thunk calls.
(define (plum/batch-run verb names thunk)
  (let loop ((names names) (ok 0) (errs '()))
    (cond
      ((null? names)
       (log! 'info
             (string-append "PLUM: "
                            (number->string ok) " " verb
                            " — "
                            (number->string (length errs)) " failed"))
       (for-each (lambda (e) (log! 'error e)) (reverse errs))
       ok)
      (else
       (let ((name (car names)))
         (log! 'info (string-append "PLUM: " verb " " name))
         (with-handler
           (lambda (err)
             (loop (cdr names) ok
                   (cons (string-append "  " name ": " (to-string err)) errs)))
           (begin
             (thunk name)
             (loop (cdr names) (+ ok 1) errs))))))))
