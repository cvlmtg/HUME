;;; core:plum/themes.scm — third-party THEME install pipeline. Clones a
;;; GitHub repo's `themes/*.toml` into `<data>/themes/`, the data-dir search
;;; tier `hume-editor`'s theme loader and `:theme <Tab>` completer already
;;; read (see README's "Theme install" section for the full design).

(require "lib.scm")

;; ── Path helpers ──────────────────────────────────────────────────────────────

(define (plum/themes-dir)
  (path-join (data-dir) "themes"))

;;; Clones live under a `sources/` subdirectory of `<data>/themes/` — never
;;; scanned by the Rust-side theme loader or `:theme <Tab>` completer (both
;;; only ever glob `*.toml` files, non-recursively), so a repo checkout
;;; sitting there is invisible to both. This is what lets install/update/
;;; list/remove work with no separate state file: the clone itself is the
;;; provenance record for which repo a given `.toml` came from.
(define (plum/theme-sources-dir)
  (path-join (plum/themes-dir) "sources"))

(define (plum/theme-src-dir slug)
  (path-join (plum/theme-sources-dir) slug))

;; ── Slug validation ────────────────────────────────────────────────────────────

;;; Validate a user-typed "user/repo" GitHub slug for `cmd`, raising a
;;; message naming `cmd` and `slug` on anything else. Unlike a plugin name
;;; (always pre-validated by declared-plugins from init.scm) or a grammar
;;; name (always from the fixed catalog), this value is typed directly at
;;; the command line and reaches `path-join` and `git clone` — the same
;;; class of untrusted input `plum/safe-segment?` (lib.scm) exists to guard.
(define (plum/parse-slug cmd slug)
  (unless (string? slug)
    (error (string-append cmd ": expected a \"user/repo\" slug, got " (to-string slug))))
  (let ((parts (split-many slug "/")))
    (unless (and (= (length parts) 2)
                 (not (equal? (car parts) ""))
                 (not (equal? (cadr parts) ""))
                 (plum/safe-segment? (car parts))
                 (plum/safe-segment? (cadr parts)))
      (error (string-append cmd ": \"" slug "\" is not a valid \"user/repo\" slug"))))
  slug)

;; ── Theme-file discovery and sync ─────────────────────────────────────────────

;;; Sorted `.toml` stems in the installed clone's `themes/` directory for
;;; `slug`, or '() if that directory doesn't exist (repo not yet installed,
;;; or it lost the directory upstream).
(define (plum/repo-theme-names slug)
  (let ((tdir (path-join (plum/theme-src-dir slug) "themes")))
    (if (not (path-exists? tdir))
        '()
        (let* ((suffix ".toml")
               (slen (string-length suffix)))
          (map (lambda (f) (substring f 0 (- (string-length f) slen)))
               (filter (lambda (f) (and (ends-with? f suffix)
                                        (> (string-length f) slen)))
                       (sort (map file-name (read-dir tdir)) string<?)))))))

;;; Copy `slug`'s current `themes/*.toml` into `<data>/themes/`, pruning any
;;; `old-names` no longer present. The single sync step shared by install
;;; and update. Raises if the repo has no `themes/*.toml` at all — silently
;;; succeeding would install nothing while still reporting success. Returns
;;; the new name list.
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
        (call! "stdlib/write-file"
               (path-join (plum/themes-dir) (string-append name ".toml"))
               (plum/read-file (path-join (plum/theme-src-dir slug) "themes" (string-append name ".toml")))))
      names)
    names))

;;; "user/repo" for every <data>/themes/sources/<user>/<repo>/ leaf holding
;;; a themes/ directory.
(define (plum/installed-theme-repos)
  (plum/two-level-repos (plum/theme-sources-dir) "themes"))

;; ── Commands ──────────────────────────────────────────────────────────────────

(define-typed-command! "plum-install-theme"
  "Install (or reinstall) a theme repo's themes/*.toml by \"user/repo\" GitHub slug, always from a clean re-clone."
  (lambda (arg)
    (let* ((slug (plum/parse-slug "plum-install-theme" arg))
           (src-dir (plum/theme-src-dir slug))
           (old-names (plum/repo-theme-names slug)))
      (log! 'info (string-append "PLUM: installing theme repo " slug))
      ;; git clone refuses a non-empty dest — clear any stale clone first.
      ;; A failure below (bad repo, no themes/*.toml) leaves this purge as
      ;; the repair path: the clone is a harmless leftover, overwritten by
      ;; the same purge on the next :plum-install-theme attempt — no
      ;; catch-and-cleanup here, see README's "Theme install" section for
      ;; why (Steel 0.8.2 catch-cleanup-reraise footgun around a
      ;; native-raising call).
      (call! "stdlib/delete-dir" src-dir)
      (plum/run! "git" (list "clone" "--" (string-append "https://github.com/" slug ".git") src-dir))
      (let ((names (plum/sync-theme-files! slug old-names)))
        (log! 'info (string-append "PLUM: installed " slug ": " (string-join names ", ")))
        (log! 'info (string-append "PLUM: run :theme " (car names) " to try it"))))))

(define-typed-command! "plum-update-themes"
  "Run git pull in every installed theme repo and re-sync its themes/*.toml."
  (lambda ()
    (let ((installed (plum/installed-theme-repos)))
      (if (null? installed)
          (log! 'info "PLUM: no installed theme repos to update")
          (plum/batch-run "updated theme repo" installed
            (lambda (slug)
              (let ((old-names (plum/repo-theme-names slug)))
                (plum/run! "git" (list "pull") #:cwd (plum/theme-src-dir slug))
                (plum/sync-theme-files! slug old-names))))))))

(define-typed-command! "plum-list-themes"
  "Log installed theme repos and the theme names each provides, plus any unmanaged .toml files in <data>/themes/."
  (lambda ()
    (let ((installed (plum/installed-theme-repos)))
      (if (null? installed)
          (log! 'info "PLUM: no theme repos installed")
          (for-each
            (lambda (slug)
              (log! 'info (string-append "PLUM: " slug ": " (string-join (plum/repo-theme-names slug) ", "))))
            installed))
      (let* ((managed (apply append (map plum/repo-theme-names installed)))
             (all-toml (if (path-exists? (plum/themes-dir))
                           (let* ((suffix ".toml") (slen (string-length suffix)))
                             (map (lambda (f) (substring f 0 (- (string-length f) slen)))
                                  (filter (lambda (f) (and (ends-with? f suffix)
                                                           (> (string-length f) slen)))
                                          (sort (map file-name (read-dir (plum/themes-dir))) string<?))))
                           '()))
             (unmanaged (filter (lambda (n) (not (member n managed))) all-toml)))
        (unless (null? unmanaged)
          (log! 'info (string-append "PLUM unmanaged: " (string-join unmanaged ", "))))))))

(define-typed-command! "plum-remove-theme"
  "Remove an installed theme repo's themes/*.toml and its clone, by \"user/repo\" GitHub slug."
  (lambda (arg)
    (let* ((slug (plum/parse-slug "plum-remove-theme" arg))
           (names (plum/repo-theme-names slug)))
      (if (null? names)
          (log! 'info (string-append "PLUM: " slug " is not installed"))
          (begin
            (for-each
              (lambda (name)
                (call! "stdlib/delete-file" (path-join (plum/themes-dir) (string-append name ".toml"))))
              names)
            (call! "stdlib/delete-dir" (plum/theme-src-dir slug))
            (log! 'info (string-append "PLUM: removed " slug ": " (string-join names ", "))))))))
