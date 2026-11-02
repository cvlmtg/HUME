;;; core:plum/themes.scm — third-party THEME install pipeline. See
;;; docs/repos.md.

(require "lib/helpers.scm")

;; ── Path helpers ──────────────────────────────────────────────────────────────

(define (plum/themes-dir)
  (path-join (data-dir) "themes"))

;;; Clones live under a `sources/` subdirectory — see docs/repos.md's "Layout and discovery".
(define (plum/theme-sources-dir)
  (path-join (plum/themes-dir) "sources"))

(define (plum/theme-src-dir slug)
  (path-join (plum/theme-sources-dir) slug))

(define (plum/theme-file dir name)
  (path-join dir (string-append name ".toml")))

;; ── Slug validation ────────────────────────────────────────────────────────────

;;; Returns the slug, or #f after reporting a status message — see
;;; docs/repos.md's "Slug validation" for the malformed-vs-unsafe error-routing distinction.
(define (plum/parse-slug cmd slug)
  (let ((parts (if (string? slug) (split-many slug "/") '())))
    (cond
      ((not (and (= (length parts) 2)
                 (not (equal? (car parts) ""))
                 (not (equal? (cadr parts) ""))))
       (log! 'info (plum/msg "expected a \"user/repo\" slug, e.g. :" cmd " cvlmtg/everforest.hume"))
       #f)
      ((not (and (safe-path-segment? (car parts))
                 (safe-path-segment? (cadr parts))))
       (error (string-append "\"" slug "\" is not a valid \"user/repo\" slug")))
      (else slug))))

;; ── Theme-file discovery and sync ─────────────────────────────────────────────

(define (plum/theme-names-in dir)
  (stems-with-suffix (path-join dir "themes") ".toml"))

(define (plum/repo-theme-names slug)
  (plum/theme-names-in (plum/theme-src-dir slug)))

;;; The theme names the clone in `dir` provides. Raises if it has no `themes/*.toml`.
(define (plum/require-theme-names dir)
  (let ((names (plum/theme-names-in dir)))
    (when (null? names)
      (error "no themes/*.toml in the repo"))
    names))

;;; `((repo . names) ...)` for the installed repos other than `slug`, in slug order;
;;; see docs/repos.md's "Name collisions".
(define (plum/other-theme-repos slug)
  (map (lambda (other) (cons other (plum/repo-theme-names other)))
       (filter (lambda (other) (not (equal? other slug)))
               (plum/installed-theme-repos))))

(define (plum/repo-themes-dir slug)
  (path-join (plum/theme-src-dir slug) "themes"))

(define (plum/theme-text dir name)
  (call-with-input-file (plum/theme-file dir name) read-port-to-string))

(define (plum/copy-theme-file! slug name)
  (call! "stdlib/write-file!"
         (plum/theme-file (plum/themes-dir) name)
         (plum/theme-text (plum/repo-themes-dir slug) name)))

;;; For each of `names`, keeps the copy in place when one of `others`, a
;;; `plum/other-theme-repos` list, ships the same text, else copies the first of `others`
;;; that ships it, else deletes it. See docs/repos.md's "Name collisions".
(define (plum/drop-theme-names! names others)
  (for-each
    (lambda (name)
      (let ((shippers (map car (filter (lambda (other) (member name (cdr other))) others)))
            (installed (plum/theme-file (plum/themes-dir) name)))
        (cond
          ((null? shippers) (call! "stdlib/delete-file!" installed))
          ((not (and (path-exists? installed)
                     (let ((text (plum/theme-text (plum/themes-dir) name)))
                       (call! "stdlib/find"
                              (lambda (slug)
                                (equal? text (plum/theme-text (plum/repo-themes-dir slug) name)))
                              shippers))))
           (plum/copy-theme-file! (car shippers) name)))))
    names))

;;; The sync step shared by install and update. Raises if the repo has no `themes/*.toml` at all.
(define (plum/sync-theme-files! slug old-names)
  (let ((names (plum/require-theme-names (plum/theme-src-dir slug)))
        (others (plum/other-theme-repos slug)))
    (plum/drop-theme-names! (filter (lambda (old) (not (member old names))) old-names) others)
    (for-each
      (lambda (name)
        (let ((shadowed (map car (filter (lambda (other) (member name (cdr other))) others))))
          (unless (null? shadowed)
            (log! 'warning (plum/msg slug "'s theme \"" name "\" shadows the same name from "
                                     (string-join shadowed ", ")))))
        (plum/copy-theme-file! slug name))
      names)
    names))

;;; Marker is `.git`, not `themes` — see docs/repos.md's "Layout and discovery".
(define (plum/installed-theme-repos)
  (plum/two-level-repos (plum/theme-sources-dir) ".git"))

;; ── Commands ──────────────────────────────────────────────────────────────────

(define-typed-command! "plum-install-theme"
  "Install (or reinstall) a theme repo's themes/*.toml by \"user/repo\" GitHub slug, always from a clean re-clone."
  (lambda (pane arg)
    (plum/report-failure! "install"
      (lambda ()
        (let ((slug (plum/parse-slug "plum-install-theme" arg)))
          (when slug
            (let ((old-names (plum/repo-theme-names slug)))
              (print! (plum/msg "installing theme repo " slug))
              (plum/install-repo! (plum/github-url slug) (plum/theme-src-dir slug) #f
                #:build plum/require-theme-names)
              (let ((names (plum/sync-theme-files! slug old-names)))
                (print! (plum/msg "installed " slug ": " (string-join names ", ")))
                (log! 'info (plum/msg "run :theme " (car names) " to try it")))))))))
  #:inline-output #t)

(define-typed-command! "plum-update-themes"
  "Bring every installed theme repo to its latest commit and re-sync its themes/*.toml."
  (lambda ()
    (plum/batch-run! "updated theme repo" (plum/installed-theme-repos)
      (lambda (slug)
        (let ((old-names (plum/repo-theme-names slug)))
          (plum/git-sync! (plum/theme-src-dir slug) #f)
          (plum/sync-theme-files! slug old-names)))
      #:none "no installed theme repos to update"
      #:progress "updating theme repo"))
  #:inline-output #t)

(define-typed-command! "plum-list-themes"
  "Show installed theme repos and the theme names each provides, plus any unmanaged .toml files in <data>/themes/."
  (lambda (pane)
    (let* ((per-repo (map (lambda (slug) (cons slug (plum/repo-theme-names slug)))
                          (plum/installed-theme-repos)))
           (managed (call! "stdlib/concat" (map cdr per-repo)))
           (unmanaged (filter (lambda (n) (not (member n managed)))
                              (stems-with-suffix (plum/themes-dir) ".toml"))))
      (call! "stdlib/show-report!" pane "plum-themes"
        (list (call! "stdlib/report-section" "Installed theme repos"
                (map (lambda (p)
                       (string-append (car p) ": "
                         (if (null? (cdr p)) "no themes" (string-join (cdr p) ", "))))
                     per-repo)
                #f)
              (call! "stdlib/report-section" "Unmanaged themes" unmanaged #f))))))

;;; `:plum-remove-theme`'s own completion universe — see README.md's "Commands".
(register-completion-source! "plum:themes"
  (lambda (id input cursor)
    (completion-emit! id (plum/installed-theme-repos)))
  #:target 'minibuf #:match 'string)

(define-typed-command! "plum-remove-theme"
  "Remove an installed theme repo's themes/*.toml and its clone, by \"user/repo\" GitHub slug."
  (lambda (pane arg)
    (plum/report-failure! "remove"
      (lambda ()
        (let ((slug (plum/parse-slug "plum-remove-theme" arg)))
          (when slug
            (let ((src-dir (plum/theme-src-dir slug))
                  (names   (plum/repo-theme-names slug)))
              ;; "Installed" is the clone existing, not still holding `themes/*.toml` —
              ;; see `plum/installed-theme-repos`'s marker.
              (if (not (path-exists? src-dir))
                  (log! 'info (plum/msg slug " is not installed"))
                  (begin
                    (plum/drop-theme-names! names (plum/other-theme-repos slug))
                    (call! "stdlib/delete-dir!" src-dir)
                    (log! 'info (plum/msg "removed " slug ": " (string-join names ", ")))))))))))
  #:complete "plum:themes")
