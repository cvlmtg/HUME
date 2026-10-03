;;; core:plum/lib.scm

(provide plum/require-stdlib! plum/batch-run! plum/read-file plum/two-level-repos
         plum/clone-github! plum/git-pull!)

(define (plum/require-stdlib!)
  (unless (member "core:stdlib" (declared-plugins))
    (error "core:plum: requires core:stdlib — add (load-plugin! \"core:stdlib\") before (load-plugin! \"core:plum\")")))

;; ── Two-level repo discovery ──────────────────────────────────────────────────

;;; Walk `root`/<user>/<repo>/ and return "user/repo" strings for every leaf
;;; containing `marker` — see README.md's "How it works" and "Theme install".
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
;; See README.md's "Output model" and `hume_platform::process::run_inline_output`'s own doc.

;;; `--` guards against a slug-derived URL `git` might otherwise read as a flag.
(define (plum/clone-github! slug dest)
  (run-inline-output! "git" (list "clone" "--" (string-append "https://github.com/" slug ".git") dest)))

;;; git pull in `dir` — the update-side shape every install command's
;;; refresh path shares (`plum-update-plugins`, `plum-update-themes`).
(define (plum/git-pull! dir)
  (run-inline-output! "git" (list "pull") #:cwd dir))

;; ── Filesystem helpers ────────────────────────────────────────────────────────
;; Thin wrappers over Steel's `steel/filesystem`/`steel/ports`.

;;; Full contents of the file at `path`, as a string.
(define (plum/read-file path)
  (let ([port (open-input-file path)])
    (let ([content (read-port-to-string port)])
      (close-input-port port)
      content)))

;; ── Batch runner ──────────────────────────────────────────────────────────────

;;; Runs `thunk` on each of `names`, collecting errors rather than aborting.
(define (plum/batch-run! verb names thunk)
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
         ;; `displayln`, not `log!` — see README.md's "Output model".
         (displayln (string-append "PLUM: " verb " " name))
         (with-handler
           (lambda (err)
             (loop (cdr names) ok
                   (cons (string-append "  " name ": " (to-string err)) errs)))
           (begin
             (thunk name)
             (loop (cdr names) (+ ok 1) errs))))))))
