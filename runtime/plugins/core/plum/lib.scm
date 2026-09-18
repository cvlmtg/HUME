;;; core:plum/lib.scm

(provide plum/batch-run plum/read-file plum/two-level-repos
         plum/clone-github!)

;; ── Two-level repo discovery ──────────────────────────────────────────────────

;;; Walk `root`/<user>/<repo>/ and return "user/repo" strings for every leaf
;;; containing `marker` — shared by plugin discovery (`marker` "plugin.scm")
;;; and theme-repo discovery (`marker` ".git", so a repo stays discoverable
;;; even after upstream drops its `themes/` directory).
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
;; All of PLUM's network commands are `#:inline-output` — git's own progress
;; prints live instead of vanishing into a captured-but-unread stdout, and a
;; credential prompt lands on a real terminal instead of hanging invisibly
;; behind the alt-screen. `run-inline-output!` (core builtin) already raises
;; on nonzero exit, naming `cmd` and the exit code — no wrapper needed here.

;;; git clone the GitHub repo named by "user/repo" `slug` into `dest`. `--`
;;; guards against a slug-derived URL `git` might otherwise read as a flag.
(define (plum/clone-github! slug dest)
  (run-inline-output! "git" (list "clone" "--" (string-append "https://github.com/" slug ".git") dest)))

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
         ;; `displayln`, not `log!` — this line must paint while the batch is
         ;; still running. `log!` only buffers until the whole command
         ;; returns, then collapses into one `status_msg` slot (see
         ;; `hume-editor/src/editor/message_log.rs`'s `report`), so every
         ;; per-item line but the last would be silently lost. `displayln` is
         ;; gated shut for a non-`#:inline-output` caller
         ;; (`plum-cleanup-plugins`, `plum-cleanup-grammars`), which is fine —
         ;; those finish instantly and keep their summary `log!` line below.
         (displayln (string-append "PLUM: " verb " " name))
         (with-handler
           (lambda (err)
             (loop (cdr names) ok
                   (cons (string-append "  " name ": " (to-string err)) errs)))
           (begin
             (thunk name)
             (loop (cdr names) (+ ok 1) errs))))))))
