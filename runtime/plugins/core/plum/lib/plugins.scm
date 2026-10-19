;;; core:plum/plugins.scm

(require "helpers.scm")

;; ── Path helpers ──────────────────────────────────────────────────────────────

(define (plum/plugins-dir)
  (path-join (data-dir) "plugins"))

(define (plum/plugin-dir name)
  (path-join (plum/plugins-dir) name))

;; ── Installed plugin discovery ────────────────────────────────────────────────

(define (plum/installed-plugins)
  (plum/two-level-repos (plum/plugins-dir) "plugin.scm"))

;; ── Set operations ────────────────────────────────────────────────────────────

;;; core:* plugins excluded — they're bundled, never installed by PLUM.
(define (plum/missing-plugins declared installed)
  (filter (lambda (name) (and (not (starts-with? name "core:"))
                               (not (member name installed))))
          declared))

(define (plum/orphan-plugins declared installed)
  (filter (lambda (name) (not (member name declared))) installed))

;; ── Commands ──────────────────────────────────────────────────────────────────

(define-typed-command! "plum-install-plugins"
  "Install all declared plugins that are not yet on disk."
  (lambda ()
    (let ((missing (plum/missing-plugins (declared-plugins) (plum/installed-plugins))))
      (if (null? missing)
          (log! 'info "PLUM: nothing to install")
          (let ((n (plum/batch-run! "installed" missing
                     (lambda (name)
                       (let ((ref (declared-plugin-ref name)))
                         (plum/clone-github! name (plum/plugin-dir name))
                         (when ref (plum/git-sync! (plum/plugin-dir name) ref)))))))
            (when (> n 0)
              (log! 'info "PLUM: run :reload-config to activate the newly installed plugins"))))))
  #:inline-output #t)

(define-typed-command! "plum-cleanup-plugins"
  "Remove on-disk plugins that are no longer declared in init.scm."
  (lambda ()
    (let ((orphans (plum/orphan-plugins (declared-plugins) (plum/installed-plugins))))
      (if (null? orphans)
          (log! 'info "PLUM: nothing to remove")
          (plum/batch-run! "removed" orphans
            (lambda (name) (call! "stdlib/delete-dir!" (plum/plugin-dir name))))))))

(define-typed-command! "plum-update-plugins"
  "Bring every installed third-party plugin to its #:ref, or its latest commit."
  (lambda ()
    (let ((installed (plum/installed-plugins)))
      (if (null? installed)
          (log! 'info "PLUM: no installed plugins to update")
          (let ((n (plum/batch-run! "updated" installed
                     (lambda (name)
                       (plum/git-sync! (plum/plugin-dir name) (declared-plugin-ref name))))))
            (when (> n 0)
              (log! 'info "PLUM: run :reload-config to pick up the updated plugins"))))))
  #:inline-output #t)

(define-typed-command! "plum-list-plugins"
  "Show the declared, installed, orphan, and missing plugin lists."
  (lambda (pane)
    (let ((declared (declared-plugins))
          (installed (plum/installed-plugins)))
      (call! "stdlib/show-report!" pane "plum-plugins"
        (list (call! "stdlib/report-section" "Declared plugins" declared #f)
              (call! "stdlib/report-section" "Installed plugins" installed #f)
              (call! "stdlib/report-section" "Orphan plugins"
                (plum/orphan-plugins declared installed)
                "remove with :plum-cleanup-plugins")
              (call! "stdlib/report-section" "Missing plugins"
                (plum/missing-plugins declared installed)
                "install with :plum-install-plugins"))))))
