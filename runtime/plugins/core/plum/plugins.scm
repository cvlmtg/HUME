;;; core:plum/plugins.scm

(require "lib.scm")

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
(define (plum/missing-plugins)
  (let ((installed (plum/installed-plugins)))
    (filter (lambda (name) (and (not (starts-with? name "core:"))
                                 (not (member name installed))))
            (declared-plugins))))

(define (plum/orphan-plugins)
  (let ((declared (declared-plugins)))
    (filter (lambda (name) (not (member name declared)))
            (plum/installed-plugins))))

;; ── Commands ──────────────────────────────────────────────────────────────────

(define-typed-command! "plum-install-plugins"
  "Install all declared plugins that are not yet on disk."
  (lambda ()
    (let ((missing (plum/missing-plugins)))
      (if (null? missing)
          (log! 'info "PLUM: nothing to install")
          (let ((n (plum/batch-run "installed" missing
                     (lambda (name) (plum/clone-github! name (plum/plugin-dir name))))))
            (when (> n 0)
              (log! 'info "PLUM: run :reload-config to activate the newly installed plugins")))))))

(define-typed-command! "plum-cleanup-plugins"
  "Remove on-disk plugins that are no longer declared in init.scm."
  (lambda ()
    (let ((orphans (plum/orphan-plugins)))
      (if (null? orphans)
          (log! 'info "PLUM: nothing to remove")
          (plum/batch-run "removed" orphans
            (lambda (name) (call! "stdlib/delete-dir" (plum/plugin-dir name))))))))

(define-typed-command! "plum-update-plugins"
  "Run git pull in every installed third-party plugin directory."
  (lambda ()
    (let ((installed (plum/installed-plugins)))
      (if (null? installed)
          (log! 'info "PLUM: no installed plugins to update")
          (let ((n (plum/batch-run "updated" installed
                     (lambda (name) (plum/run! "git" (list "pull") #:cwd (plum/plugin-dir name))))))
            (when (> n 0)
              (log! 'info "PLUM: run :reload-config to pick up the updated plugins")))))))

(define-typed-command! "plum-list-plugins"
  "Log the declared, installed, orphan, and missing plugin lists."
  (lambda ()
    (let ((declared   (declared-plugins))
          (installed  (plum/installed-plugins))
          (orphans    (plum/orphan-plugins))
          (missing    (plum/missing-plugins)))
      (log! 'info (string-append "PLUM declared:   " (string-join declared ", ")))
      (log! 'info (string-append "PLUM installed:  " (string-join installed ", ")))
      (log! 'info (string-append "PLUM orphan:     " (string-join orphans ", ")))
      (log! 'info (string-append "PLUM missing:    " (string-join missing ", "))))))
