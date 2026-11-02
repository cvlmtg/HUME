;;; core:plum/lib/helpers.scm

(provide plum/msg plum/report-failure! plum/batch-run! plum/two-level-repos
         plum/staging-dir plum/github-url plum/staged-install! plum/install-repo! plum/git-sync!)

;; ── Messages ──────────────────────────────────────────────────────────────────

;;; A line PLUM logs or prints; see docs/architecture.md's "Shared module".
(define (plum/msg . parts)
  (apply string-append "PLUM: " parts))

;;; Runs `thunk`, logging an error it raises as `what` having failed.
(define (plum/report-failure! what thunk)
  (with-handler
    (lambda (err) (log! 'error (plum/msg what " failed: " (error-text err))))
    (thunk)))

;; ── Two-level repo discovery ──────────────────────────────────────────────────

;;; Walk `root`/<user>/<repo>/ and return "user/repo" strings for every leaf
;;; containing `marker` — see docs/repos.md's "Plugin discovery" and "Layout and discovery".
(define (plum/two-level-repos root marker)
  (if (not (path-exists? root))
      '()
      (call! "stdlib/concat"
             (map (lambda (user)
                    (let ((udir (path-join root user)))
                      (map (lambda (repo) (string-append user "/" repo))
                           (filter (lambda (repo)
                                     (path-exists? (path-join udir repo marker)))
                                   (call! "stdlib/list-subdirs" udir)))))
                  (call! "stdlib/list-subdirs" root)))))

;; ── Clone and checkout ────────────────────────────────────────────────────────

(define (plum/github-url slug)
  (string-append "https://github.com/" slug ".git"))

(define (plum/staging-dir)
  (path-join (data-dir) "plum-staging"))

;;; Fills the empty directory `stage` with `fill`, given `stage`, then moves it to `dest`,
;;; replacing `dest` only once `fill` has returned. `stage` is deleted on failure.
(define (plum/staged-install! stage dest fill)
  (call! "stdlib/delete-dir!" stage)
  (create-directory! stage)
  (with-handler
    (lambda (err)
      (call! "stdlib/delete-dir!" stage)
      (raise-error err))
    (fill stage)
    (replace-dir! stage dest)))

;;; Installs `url` into `dest` at `ref`, replacing `dest` only once the new clone is
;;; checked out and `build`, given the clone's directory, has returned.
;;; See docs/repos.md's "Pinned plugins".
(define (plum/install-repo! url dest ref #:build [build #f])
  (plum/staged-install!
    (path-join (plum/staging-dir) (file-name (parent-name dest)) (file-name dest)) dest
    (lambda (stage)
      (plum/git! stage (list "init" "-q"))
      (plum/git! stage (list "remote" "add" "--" "origin" url))
      (plum/git! stage (list "config" "remote.origin.promisor" "true"))
      (plum/git! stage (list "config" "remote.origin.partialclonefilter" "blob:none"))
      (plum/git-sync! stage ref)
      (when build (build stage)))))

;;; Checks the clone in `dir` out at origin's `ref`, or its default branch for #f.
;;; See docs/repos.md's "Pinned plugins".
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

;;; See docs/architecture.md's "Output model".
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

;; ── Batch runner ──────────────────────────────────────────────────────────────

;;; See docs/architecture.md's "Shared module".
(define (plum/batch-run! verb names thunk #:none [none #f] #:skip [skip #f] #:progress [progress #f])
  (if (and none (null? names))
      (begin (log! 'info (plum/msg none)) 0)
      (let loop ((names names) (ok 0) (failed '()) (errs '()))
        (cond
          ((null? names)
           (log! 'info (plum/msg (number->string ok) " " verb
                                 " — " (number->string (length errs)) " failed"))
           (for-each (lambda (e) (log! 'error e)) (reverse errs))
           ok)
          (else
           (let* ((name (car names))
                  (fail (lambda (reason)
                          (loop (cdr names) ok (cons name failed)
                                (cons (plum/msg name ": " reason) errs))))
                  (skipped (and skip (skip name failed))))
             (if skipped
                 (fail skipped)
                 (begin
                   ;; See docs/architecture.md's "Output model".
                   (when progress (print! (plum/msg progress " " name)))
                   (let ((reason (with-handler (lambda (err) (error-text err))
                                   (thunk name)
                                   #f)))
                     (if reason
                         (fail reason)
                         (loop (cdr names) (+ ok 1) failed errs)))))))))))
