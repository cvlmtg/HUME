;;; core:git-diff — plugin.scm (see manifest.scm and README.md "Usage" and
;;; "File layout").

(require "state.scm")
(require "diff.scm")
(require "branch.scm")
(require "render.scm")

(unless (member "core:stdlib" (declared-plugins))
  (error "core:git-diff: requires core:stdlib — (declare-plugin \"core:stdlib\") or (load-plugin \"core:stdlib\") before (load-plugin \"core:git-diff\")"))

;; ── Config ────────────────────────────────────────────────────────────────────

(define git-diff/cfg (plugin-config))
(define git-diff/signs-default (call! "stdlib/config-boolean" "core:git-diff" git-diff/cfg "signs" #t))
(define git-diff/inline-default (call! "stdlib/config-boolean" "core:git-diff" git-diff/cfg "inline" #f))
(define git-diff/ref (call! "stdlib/config-string" "core:git-diff" git-diff/cfg "ref" "HEAD"))

;;; A runtime override (`state.scm`'s "ref" field) wins over the config
;;; default; an untracked buffer falls through to it.
(define (git-diff/buffer-ref bid)
  (let ([entry (git-diff/buffer-entry bid)])
    (or (and entry (hash-ref entry "ref")) git-diff/ref)))

;;; `'()` for an untracked buffer.
(define (git-diff/buffer-hunks bid)
  (let ([entry (git-diff/buffer-entry bid)])
    (if entry (hash-ref entry "hunks") '())))

;; ── Lifecycle ─────────────────────────────────────────────────────────────────

(register-hook! 'on-buffer-open
  (lambda (bid)
    (git-diff/init-buffer! bid git-diff/signs-default git-diff/inline-default)
    (git-diff/schedule-refresh! bid (git-diff/buffer-ref bid))))

(register-hook! 'on-buffer-enter
  (lambda (bid) (git-diff/schedule-branch-refresh! bid)))

;;; The branch fetch is gated on `"steel:git-branch"` being placed
;;; (`branch.scm`'s `branch-element-placed?`) — this is what drives it in
;;; the moment a user places it, rather than waiting for the next focus
;;; change or save. Both `configure-statusline!` and `:set global
;;; statusline=…` funnel through this one `on-option-change` raise site.
(register-hook! 'on-option-change
  (lambda (key value)
    (when (equal? key "statusline")
      (git-diff/schedule-branch-refresh! (current-buffer)))))

(register-hook! 'on-text-changed
  (lambda (bid) (git-diff/schedule-refresh! bid (git-diff/buffer-ref bid))))

(register-hook! 'on-buffer-save
  (lambda (bid)
    (git-diff/cancel-fetch! bid)
    (git-diff/entry-set! bid "ref-text" #f)
    (git-diff/schedule-refresh! bid (git-diff/buffer-ref bid))
    (git-diff/schedule-branch-refresh! bid)))

(register-hook! 'on-buffer-close
  (lambda (bid)
    (git-diff/cancel-fetch! bid)
    (git-diff/cancel-branch-fetch! bid)
    (git-diff/remove-buffer! bid)))

;; ── Commands ──────────────────────────────────────────────────────────────────

;;; Shared body for both toggles below — see README's "Ref handling" for
;;; the ref-argument contract.
(define (git-diff/run-toggle! bid key label arg)
  (let ([enabled?
         (if (string? arg)
             (begin (git-diff/ensure-entry! bid)
                    (git-diff/entry-set! bid key #t)
                    (git-diff/entry-set! bid "ref" arg)
                    (git-diff/entry-set! bid "ref-text" #f)
                    #t)
             (git-diff/toggle-flag! bid key))])
    (if enabled?
        (begin
          (git-diff/render-for! key bid (git-diff/buffer-hunks bid))
          (git-diff/force-refresh! bid (git-diff/buffer-ref bid)))
        (git-diff/render-for! key bid '()))
    (log! 'info (if enabled?
                    (string-append "git-diff: " label " on (" (git-diff/buffer-ref bid) ")")
                    (string-append "git-diff: " label " off")))))

(define-typed-command! "toggle-git-signs"
  "Toggle gutter +/-/~ signs for the current buffer's git diff. Optional argument: a git ref to diff against, e.g. :toggle-git-signs HEAD~2 (default: the `ref` config value, shared with toggle-inline-diff)."
  (lambda (arg) (git-diff/run-toggle! (current-buffer) "signs?" "signs" arg)))

(define-typed-command! "toggle-inline-diff"
  "Toggle inline git diff rendering (virtual deleted lines, word highlights, background tint). Optional argument: a git ref to diff against, e.g. :toggle-inline-diff HEAD~2 (default: the `ref` config value, shared with toggle-git-signs)."
  (lambda (arg) (git-diff/run-toggle! (current-buffer) "inline?" "inline diff" arg)))
