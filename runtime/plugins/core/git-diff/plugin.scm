;;; core:git-diff — plugin.scm. See docs/architecture.md.

(require "state.scm")
(require "diff.scm")
(require "branch.scm")
(require "render.scm")

(unless (member "core:stdlib" (declared-plugins))
  (error "core:git-diff: requires core:stdlib — add (load-plugin! \"core:stdlib\") before (load-plugin! \"core:git-diff\")"))

;; ── Config ────────────────────────────────────────────────────────────────────

(define git-diff/cfg (plugin-config))
(define git-diff/signs-default (call! "stdlib/config-boolean" "core:git-diff" git-diff/cfg "signs" #t))
(define git-diff/inline-default (call! "stdlib/config-boolean" "core:git-diff" git-diff/cfg "inline" #f))
(define git-diff/ref (call! "stdlib/config-string" "core:git-diff" git-diff/cfg "ref" "HEAD"))

;; ── Lifecycle ─────────────────────────────────────────────────────────────────

(register-hook! 'on-buffer-open
  (lambda (pane)
    (git-diff/init-buffer! pane git-diff/signs-default git-diff/inline-default)
    (git-diff/schedule-refresh! pane git-diff/ref)))

(register-hook! 'on-buffer-enter
  (lambda (pane)
    (git-diff/invalidate-ref! pane git-diff/ref)
    (git-diff/schedule-branch-refresh! pane)))

;;; Drives the branch fetch — see docs/pipeline.md's "Branch tracking (`branch.scm`)".
(register-hook! 'on-option-change
  (lambda (key value)
    (when (equal? key "statusline")
      (git-diff/schedule-branch-refresh! (focused-pane)))))

(register-hook! 'on-text-changed
  (lambda (pane) (git-diff/schedule-refresh! pane git-diff/ref)))

(register-hook! 'on-buffer-save
  (lambda (pane)
    (git-diff/invalidate-ref! pane git-diff/ref)
    (git-diff/schedule-branch-refresh! pane)))

(register-hook! 'on-buffer-close
  (lambda (pane) (git-diff/remove-buffer! pane)))

;; ── Commands ──────────────────────────────────────────────────────────────────

(define (git-diff/draw-for-source! pane source hunks)
  (when (equal? source git-diff/*source*)
    (error (string-append "git-diff/render-diff: the source \"" source "\" is core:git-diff's own")))
  (git-diff/render-diff! pane source hunks)
  (git-diff/add-cover! pane source)
  (git-diff/reconcile! pane))

(define (git-diff/release-source! pane source)
  (git-diff/render-diff! pane source '())
  (git-diff/remove-cover! pane source)
  (git-diff/refresh! pane git-diff/ref))

(define-command! "git-diff/render-diff"
  "Draw hunks inline in a buffer: deleted lines as virtual lines, word highlights and a line tint. Arguments: pane, a decoration source name, a list of hunks in the shape `diff-buffer-lines` and `buffer-revision-diff` return. The source keeps the drawing apart from every other source's; an empty list draws nothing. Until `git-diff/release-diff` releases the source, the buffer's own inline git diff is hidden; its gutter signs stay."
  git-diff/draw-for-source!)

(define-command! "git-diff/release-diff"
  "Clear what a source drew through `git-diff/render-diff` and stop hiding the buffer's own inline git diff for it. Arguments: pane, the decoration source name."
  git-diff/release-source!)

;;; `:toggle-git-signs`/`:toggle-inline-diff`'s shared completion universe — see
;;; docs/architecture.md's "Ref handling".
(register-completion-source! "git-diff:refs"
  (lambda (id input cursor)
    (let ([path (buffer-path (focused-pane))])
      (if (not path)
          (completion-emit! id '())
          (spawn-async! "git"
            '("for-each-ref" "--format=%(refname:short)" "refs/heads" "refs/tags" "refs/remotes")
            (lambda (stdout stderr exit-code)
              (completion-emit! id
                (if (= exit-code 0)
                    (filter (lambda (s) (not (equal? s ""))) (split-many stdout "\n"))
                    '())))
            #:cwd (parent-name path)))))
  #:target 'minibuf #:match 'string)

;;; Shared body for both toggles below — see docs/architecture.md's "Ref handling".
(define (git-diff/run-toggle! pane key label arg)
  (let ([enabled?
         (if (string? arg)
             (begin (git-diff/ensure-entry! pane)
                    (git-diff/entry-set! pane key #t)
                    (git-diff/entry-set! pane "ref" arg)
                    (git-diff/drop-ref-text! pane)
                    #t)
             (git-diff/toggle-flag! pane key))])
    (if enabled?
        (git-diff/force-refresh! pane git-diff/ref)
        (git-diff/reconcile! pane))
    (log! 'info (if enabled?
                    (string-append "git-diff: " label " on (" (git-diff/buffer-ref pane git-diff/ref) ")")
                    (string-append "git-diff: " label " off")))))

(define-typed-command! "toggle-git-signs"
  "Toggle gutter +/~ signs and deletion boundary marks for the current buffer's git diff. Optional argument: a git ref to diff against, e.g. :toggle-git-signs HEAD~2 (default: the `ref` config value, shared with toggle-inline-diff)."
  (lambda (pane arg) (git-diff/run-toggle! pane "signs?" "signs" arg))
  #:complete "git-diff:refs")

(define-typed-command! "toggle-inline-diff"
  "Toggle inline git diff rendering (virtual deleted lines, word highlights, background tint). Optional argument: a git ref to diff against, e.g. :toggle-inline-diff HEAD~2 (default: the `ref` config value, shared with toggle-git-signs)."
  (lambda (pane arg) (git-diff/run-toggle! pane "inline?" "inline diff" arg))
  #:complete "git-diff:refs")
