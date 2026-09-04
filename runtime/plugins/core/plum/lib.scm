;;; core:plum/lib.scm

(provide plum/batch-run plum/run! plum/read-file plum/safe-segment? plum/two-level-repos)

;; ── Path-segment validation ───────────────────────────────────────────────────

;;; Safe to use as one filesystem path segment — see README.md.
(define (plum/safe-segment? name)
  (and (not (equal? name "."))
       (not (equal? name ".."))
       (not (string-contains? name "/"))
       (not (string-contains? name "\\"))
       (not (string-contains? name ":"))
       (not (string-contains? name "\""))))

;; ── Two-level repo discovery ──────────────────────────────────────────────────

;;; Walk `root`/<user>/<repo>/ and return "user/repo" strings for every leaf
;;; containing `marker` — shared by plugin discovery (`marker` "plugin.scm")
;;; and theme-repo discovery (`marker` "themes").
(define (plum/two-level-repos root marker)
  (if (not (path-exists? root))
      '()
      (apply append
             (map (lambda (user)
                    (let ((udir (path-join root user)))
                      (map (lambda (repo) (string-append user "/" repo))
                           (filter (lambda (repo)
                                     (path-exists? (path-join udir repo marker)))
                                   (call! "stdlib/list-subdirs" udir)))))
                  (call! "stdlib/list-subdirs" root)))))

;; ── Process spawning ──────────────────────────────────────────────────────────
;; Built on core:stdlib's `stdlib/run` (call! via core:stdlib — load it first,
;; see plugin.scm's header).

;;; Raises, naming `cmd` and stderr, on nonzero exit, spawn failure, or wait
;;; failure.
(define (plum/run! cmd args #:cwd [dir #f])
  (let* ([result (call! "stdlib/run" cmd args dir)]
         [stderr (cadr result)]
         [code (caddr result)])
    (cond
      ((not code)
       (error (string-append cmd ": " stderr)))
      ((not (= code 0))
       (error (string-append cmd ": failed (exit " (number->string code) "): " (trim stderr)))))))

;; ── Filesystem helpers ────────────────────────────────────────────────────────
;; Thin wrappers over Steel's `steel/filesystem`/`steel/ports`.

;;; Full contents of the file at `path`, as a string.
(define (plum/read-file path)
  (let ([port (open-input-file path)])
    (let ([content (read-port-to-string port)])
      (close-input-port port)
      content)))

;; ── Batch runner ──────────────────────────────────────────────────────────────

;;; Runs `thunk` on each of `names`, collecting errors rather than
;;; aborting. Returns the count of successful calls.
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
