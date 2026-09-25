;;; core:plum/themes.scm — third-party THEME install pipeline. See
;;; README.md.

(require "lib.scm")

;; ── Path helpers ──────────────────────────────────────────────────────────────

(define (plum/themes-dir)
  (path-join (data-dir) "themes"))

;;; Clones live under a `sources/` subdirectory — see README.md for why.
(define (plum/theme-sources-dir)
  (path-join (plum/themes-dir) "sources"))

(define (plum/theme-src-dir slug)
  (path-join (plum/theme-sources-dir) slug))

;; ── Slug validation ────────────────────────────────────────────────────────────

;;; Returns the slug, or #f after reporting a status message — see
;;; README.md for the malformed-vs-unsafe error-routing distinction.
(define (plum/parse-slug cmd slug)
  (let ((parts (if (string? slug) (split-many slug "/") '())))
    (cond
      ((not (and (= (length parts) 2)
                 (not (equal? (car parts) ""))
                 (not (equal? (cadr parts) ""))))
       (log! 'info (string-append cmd ": expected a \"user/repo\" slug, e.g. "
                                  cmd " cvlmtg/everforest.hume"))
       #f)
      ;; `eq? #t`, not a bare truthiness check — see core:stdlib's README.
      ((not (and (eq? #t (call! "stdlib/safe-path-segment?" (car parts)))
                 (eq? #t (call! "stdlib/safe-path-segment?" (cadr parts)))))
       (error (string-append cmd ": \"" slug "\" is not a valid \"user/repo\" slug")))
      (else slug))))

;; ── Theme-file discovery and sync ─────────────────────────────────────────────

(define (plum/repo-theme-names slug)
  (stems-with-suffix (path-join (plum/theme-src-dir slug) "themes") ".toml"))

;;; Other installed repos (besides `slug`) that also provide `name` — see README.md's "Theme install".
(define (plum/theme-owned-elsewhere name slug)
  (filter (lambda (other) (and (not (equal? other slug))
                               (member name (plum/repo-theme-names other))))
          (plum/installed-theme-repos)))

;;; The sync step shared by install and update. Raises if the repo has no `themes/*.toml` at all.
(define (plum/sync-theme-files! slug old-names)
  (let ((names (plum/repo-theme-names slug)))
    (when (null? names)
      (error (string-append "plum: " slug " has no themes/*.toml — nothing to install")))
    (for-each
      (lambda (old)
        (unless (member old names)
          (call! "stdlib/delete-file" (path-join (plum/themes-dir) (string-append old ".toml")))))
      old-names)
    (for-each
      (lambda (name)
        (let ((shadowed (plum/theme-owned-elsewhere name slug)))
          (unless (null? shadowed)
            (log! 'warn (string-append "PLUM: " slug "'s theme \"" name "\" shadows the same "
                                       "name from " (string-join shadowed ", ")))))
        (call! "stdlib/write-file"
               (path-join (plum/themes-dir) (string-append name ".toml"))
               (plum/read-file (path-join (plum/theme-src-dir slug) "themes" (string-append name ".toml")))))
      names)
    names))

;;; Marker is `.git`, not `themes` — see README.md's "Theme install".
(define (plum/installed-theme-repos)
  (plum/two-level-repos (plum/theme-sources-dir) ".git"))

;; ── Commands ──────────────────────────────────────────────────────────────────

(define-typed-command! "plum-install-theme"
  "Install (or reinstall) a theme repo's themes/*.toml by \"user/repo\" GitHub slug, always from a clean re-clone."
  (lambda (pane arg)
    (let ((slug (plum/parse-slug "plum-install-theme" arg)))
      (when slug
        (let* ((src-dir (plum/theme-src-dir slug))
               (old-names (plum/repo-theme-names slug)))
          (log! 'info (string-append "PLUM: installing theme repo " slug))
          ;; Clear any stale clone first — see README.md's "Theme install".
          (call! "stdlib/delete-dir" src-dir)
          (plum/clone-github! slug src-dir)
          (let ((names (plum/sync-theme-files! slug old-names)))
            (log! 'info (string-append "PLUM: installed " slug ": " (string-join names ", ")))
            (log! 'info (string-append "PLUM: run :theme " (car names) " to try it")))))))
  #:inline-output #t)

(define-typed-command! "plum-update-themes"
  "Run git pull in every installed theme repo and re-sync its themes/*.toml."
  (lambda ()
    (let ((installed (plum/installed-theme-repos)))
      (if (null? installed)
          (log! 'info "PLUM: no installed theme repos to update")
          (plum/batch-run "updated theme repo" installed
            (lambda (slug)
              (let ((old-names (plum/repo-theme-names slug)))
                (plum/git-pull! (plum/theme-src-dir slug))
                (plum/sync-theme-files! slug old-names)))))))
  #:inline-output #t)

(define-typed-command! "plum-list-themes"
  "Log installed theme repos and the theme names each provides, plus any unmanaged .toml files in <data>/themes/."
  (lambda ()
    ;; Pair each repo with its theme names once, reused below for `managed`.
    (let ((per-repo (map (lambda (slug) (cons slug (plum/repo-theme-names slug)))
                         (plum/installed-theme-repos))))
      (if (null? per-repo)
          (log! 'info "PLUM: no theme repos installed")
          (for-each
            (lambda (p) (log! 'info (string-append "PLUM: " (car p) ": " (string-join (cdr p) ", "))))
            per-repo))
      (let* ((managed (apply append (map cdr per-repo)))
             (unmanaged (filter (lambda (n) (not (member n managed)))
                                (stems-with-suffix (plum/themes-dir) ".toml"))))
        (unless (null? unmanaged)
          (log! 'info (string-append "PLUM unmanaged: " (string-join unmanaged ", "))))))))

;;; `:plum-remove-theme`'s own completion universe — see README.md's "Commands".
(register-completion-source! "plum:themes"
  (lambda (id input cursor)
    (completion-emit! id (plum/installed-theme-repos)))
  #:target 'minibuf #:match 'string)

(define-typed-command! "plum-remove-theme"
  "Remove an installed theme repo's themes/*.toml and its clone, by \"user/repo\" GitHub slug."
  (lambda (pane arg)
    (let ((slug (plum/parse-slug "plum-remove-theme" arg)))
      (when slug
        (let ((src-dir (plum/theme-src-dir slug))
              (names   (plum/repo-theme-names slug)))
          ;; "Installed" is the clone existing, not still holding `themes/*.toml` —
          ;; see `plum/installed-theme-repos`'s marker.
          (if (not (path-exists? src-dir))
              (log! 'info (string-append "PLUM: " slug " is not installed"))
              (begin
                (for-each
                  (lambda (name)
                    (call! "stdlib/delete-file" (path-join (plum/themes-dir) (string-append name ".toml"))))
                  names)
                (call! "stdlib/delete-dir" src-dir)
                (log! 'info (string-append "PLUM: removed " slug ": " (string-join names ", ")))))))))
  #:complete "plum:themes")
