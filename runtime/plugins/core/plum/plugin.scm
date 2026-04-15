;;; core:plum — HUME's plugin manager
;;;
;;; Provides four commands for managing third-party Steel plugins:
;;;
;;;   :plum-install  — git-clone every declared plugin that is not yet on disk
;;;   :plum-cleanup  — delete every on-disk plugin that is no longer declared
;;;   :plum-update   — git pull every installed third-party plugin
;;;   :plum-list     — log declared / installed / orphan / missing lists
;;;
;;; All filesystem and git operations are sandboxed by the Rust builtins to
;;; <data>/plugins/ — PLUM cannot reach outside that directory.
;;;
;;; Usage in init.scm (add before other third-party load-plugin calls):
;;;   (load-plugin "core:plum")

;; ── Path helpers ──────────────────────────────────────────────────────────────

(define (plum/plugins-dir)
  (path-join (data-dir) "plugins"))

(define (plum/plugin-dir name)
  (path-join (plum/plugins-dir) name))

;; ── Installed plugin discovery ────────────────────────────────────────────────

;;; Walk <data>/plugins/<user>/<repo>/ and return a list of "user/repo"
;;; strings for every entry that contains a plugin.scm file.
(define (plum/installed-plugins)
  (let ((pdir (plum/plugins-dir)))
    (if (not (path-exists? pdir))
        '()
        (let user-loop ((users (list-dir pdir))
                        (result '()))
          (cond
            ((null? users)
             (reverse result))
            (else
             (let* ((user  (car users))
                    (udir  (path-join pdir user)))
               (let repo-loop ((repos (list-dir udir))
                               (acc result))
                 (cond
                   ((null? repos)
                    (user-loop (cdr users) acc))
                   (else
                    (let* ((repo (car repos))
                           (scm  (path-join udir repo "plugin.scm")))
                      (repo-loop
                        (cdr repos)
                        (if (path-exists? scm)
                            (cons (string-append user "/" repo) acc)
                            acc)))))))))))))

;; ── Set operations ────────────────────────────────────────────────────────────

;;; Plugins declared in init.scm that are not yet on disk.
(define (plum/missing-plugins)
  (let ((installed (plum/installed-plugins)))
    (filter (lambda (name) (not (member name installed)))
            (declared-plugins))))

;;; Plugins on disk that are not (or no longer) declared in init.scm.
(define (plum/orphan-plugins)
  (let ((declared (declared-plugins)))
    (filter (lambda (name) (not (member name declared)))
            (plum/installed-plugins))))

;; ── Batch runner ──────────────────────────────────────────────────────────────

;;; Run `thunk` on each name in `names`, collecting errors rather than
;;; aborting.  Logs per-item progress and a summary at the end.
;;; Returns void.
(define (plum/batch-run verb names thunk)
  (let loop ((names names) (ok 0) (errs '()))
    (cond
      ((null? names)
       (log! 'info
             (string-append "PLUM: "
                            (number->string ok) " " verb
                            " — "
                            (number->string (length errs)) " failed"))
       (for-each (lambda (e) (log! 'error e)) (reverse errs)))
      (else
       (let ((name (car names)))
         (log! 'info (string-append "PLUM: " verb " " name))
         (with-handler
           (lambda (err)
             (loop (cdr names) ok
                   (cons (string-append "  " name ": " (to-string err)) errs)))
           (begin
             (thunk name)
             (loop (cdr names) (+ ok 1) errs))))))))

;; ── Commands ──────────────────────────────────────────────────────────────────

(define-command! "plum-install"
  "Install all declared plugins that are not yet on disk."
  (lambda ()
    (let ((missing (plum/missing-plugins)))
      (if (null? missing)
          (log! 'info "PLUM: nothing to install")
          (plum/batch-run "installed" missing
            (lambda (name)
              (git-clone (string-append "https://github.com/" name ".git")
                         (plum/plugin-dir name))))))))

(define-command! "plum-cleanup"
  "Remove on-disk plugins that are no longer declared in init.scm."
  (lambda ()
    (let ((orphans (plum/orphan-plugins)))
      (if (null? orphans)
          (log! 'info "PLUM: nothing to remove")
          (plum/batch-run "removed" orphans
            (lambda (name) (delete-dir (plum/plugin-dir name))))))))

(define-command! "plum-update"
  "Run git pull in every installed third-party plugin directory."
  (lambda ()
    (let ((installed (plum/installed-plugins)))
      (if (null? installed)
          (log! 'info "PLUM: no installed plugins to update")
          (plum/batch-run "updated" installed
            (lambda (name) (git-pull (plum/plugin-dir name))))))))

(define-command! "plum-list"
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
