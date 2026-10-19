;;; core:plum/lib/helpers.scm

(provide plum/require-stdlib! plum/batch-run! plum/read-file plum/two-level-repos
         plum/staging-dir plum/github-url plum/install-repo! plum/git-sync!)

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

(define (plum/github-url slug)
  (string-append "https://github.com/" slug ".git"))

(define (plum/staging-dir)
  (path-join (data-dir) "plum-staging"))

;;; Installs `url` into `dest` at `ref`, replacing `dest` only once the new clone is
;;; checked out and `build`, given the clone's directory, has returned.
;;; See README.md's "Pinned plugins".
(define (plum/install-repo! url dest ref #:build [build #f])
  (let ((stage (path-join (plum/staging-dir) (file-name (parent-name dest)) (file-name dest))))
    (call! "stdlib/delete-dir!" stage)
    (create-directory! stage)
    ;; Why the raise follows the handler: README.md's "Pinned plugins".
    (let ((failure
            (with-handler
              (lambda (err) (to-string err))
              (begin
                (plum/git! stage (list "init" "-q"))
                (plum/git! stage (list "remote" "add" "--" "origin" url))
                (plum/git! stage (list "config" "remote.origin.promisor" "true"))
                (plum/git! stage (list "config" "remote.origin.partialclonefilter" "blob:none"))
                (plum/git-sync! stage ref)
                (when build (build stage))
                #f))))
      (when failure
        (call! "stdlib/delete-dir!" stage)
        (error failure)))
    (call! "stdlib/delete-dir!" dest)
    (create-directory! (parent-name dest))
    (rename-file-or-directory! stage dest)))

;;; Checks the clone in `dir` out at origin's `ref`, or its default branch for #f.
;;; See README.md's "Pinned plugins".
(define (plum/git-sync! dir ref)
  (let ((by-name (and ref (not (plum/commit-id? ref 40)))))
    (plum/git! dir (if by-name
                       (list "fetch" "--force" "--prune" "--tags" "origin")
                       (list "fetch" "origin" (or ref "HEAD"))))
    (plum/git! dir (list "checkout" "--detach"
                         (if by-name
                             (or (plum/resolve-ref dir ref)
                                 (error (string-append "no tag, branch or commit '" ref
                                                       "' upstream; a short commit id must be "
                                                       "unambiguous and on a branch or tag")))
                             "FETCH_HEAD")))))

(define (plum/git! dir args)
  (run-inline-output! "git" args #:cwd dir))

;;; The commit `ref` names in the fetched clone in `dir`, or #f.
(define (plum/resolve-ref dir ref)
  (let loop ((candidates (append (list (string-append "refs/tags/" ref)
                                       (string-append "refs/remotes/origin/" ref))
                                 (if (plum/commit-id? ref 4) (list ref) '()))))
    (if (null? candidates)
        #f
        (let ((result (run-capture! "git"
                        (list "rev-parse" "--verify" "--quiet"
                              (string-append (car candidates) "^{commit}"))
                        #:cwd dir)))
          (if (equal? (hash-ref result 'exit) 0)
              (trim (hash-ref result 'stdout))
              (loop (cdr candidates)))))))

;;; `min` to 40 hex digits: a commit id, abbreviated to no fewer than `min` digits.
(define (plum/commit-id? ref min)
  (let ((hex (string->list "0123456789abcdefABCDEF"))
        (n (string-length ref)))
    (and (>= n min) (<= n 40)
         (null? (filter (lambda (c) (not (member c hex))) (string->list ref))))))

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
