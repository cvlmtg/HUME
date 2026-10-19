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
    (run-inline-output! "git" (list "init" "-q") #:cwd stage)
    (run-inline-output! "git" (list "remote" "add" "--" "origin" url) #:cwd stage)
    (run-inline-output! "git" (list "config" "remote.origin.promisor" "true") #:cwd stage)
    (run-inline-output! "git" (list "config" "remote.origin.partialclonefilter" "blob:none")
                        #:cwd stage)
    (plum/git-sync! stage ref)
    (when build (build stage))
    (call! "stdlib/delete-dir!" dest)
    (create-directory! (parent-name dest))
    (rename-file-or-directory! stage dest)))

;;; Checks the clone in `dir` out at origin's `ref`, or its default branch for #f.
;;; See README.md's "Pinned plugins".
(define (plum/git-sync! dir ref)
  (if ref
      (begin
        (run-inline-output! "git" (list "fetch" "--force" "--prune" "--tags" "origin") #:cwd dir)
        (let ((id (plum/resolve-ref dir ref)))
          (unless id
            (error (string-append "no tag, branch or commit '" ref "' upstream; a short "
                                  "commit id must be unambiguous and on a branch or tag")))
          (run-inline-output! "git" (list "checkout" "--detach" id) #:cwd dir)))
      (begin
        (run-inline-output! "git" (list "fetch" "origin" "HEAD") #:cwd dir)
        (run-inline-output! "git" (list "checkout" "--detach" "FETCH_HEAD") #:cwd dir))))

;;; The commit `ref` names in the fetched clone in `dir`, or #f.
(define (plum/resolve-ref dir ref)
  (let loop ((candidates (cons (string-append "refs/tags/" ref)
                               (cons (string-append "refs/remotes/origin/" ref)
                                     (if (plum/commit-id? ref) (list ref) '())))))
    (if (null? candidates)
        #f
        (let ((result (run-capture! "git"
                        (list "rev-parse" "--verify" "--quiet"
                              (string-append (car candidates) "^{commit}"))
                        #:cwd dir)))
          (if (equal? (hash-ref result 'exit) 0)
              (trim (hash-ref result 'stdout))
              (loop (cdr candidates)))))))

;;; 4 to 40 hex digits: a full or abbreviated commit id.
(define (plum/commit-id? ref)
  (let ((hex (string->list "0123456789abcdefABCDEF"))
        (n (string-length ref)))
    (and (>= n 4) (<= n 40)
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
