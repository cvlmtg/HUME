;;; core:plum/lib.scm

(provide plum/valid-dir-entry? plum/batch-run
         plum/run! plum/list-dir plum/read-file)

;; ── Directory entry filter ────────────────────────────────────────────────────

;;; Return #t if `name` is a valid, traversable directory entry (not "." or "..").
(define (plum/valid-dir-entry? name)
  (and (not (equal? name "."))
       (not (equal? name ".."))))

;; ── Process spawning ──────────────────────────────────────────────────────────
;; `run-inline-output!` handles `#:inline-output` commands (process-group
;; safety for Ctrl+C); `plum/run!` is for everything else that runs with the
;; TUI's raw mode still on.

;;; Spawn `cmd`/`args`, capturing stdout+stderr; blocks until exit. Raises,
;;; naming `cmd` and stderr, on nonzero exit, spawn failure, or wait failure.
;;; stdin is piped and closed immediately — never inherited from HUME's own
;;; terminal, or the child's reads would race the editor's key reads.
;;; Gotcha (pinned by a permanent hume-scripting test): grab `child-stderr`
;;; before `wait`, or it returns `#f` even though the stream was piped.
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
;; Thin wrappers over Steel's `steel/filesystem`/`steel/ports`.

;;; Sorted list of basenames in `dir` (`read-dir` itself returns full paths).
(define (plum/list-dir dir)
  (sort (map file-name (read-dir dir)) string<?))

;;; Full contents of the file at `path`, as a string.
(define (plum/read-file path)
  (let ([port (open-input-file path)])
    (let ([content (read-port-to-string port)])
      (close-input-port port)
      content)))

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
