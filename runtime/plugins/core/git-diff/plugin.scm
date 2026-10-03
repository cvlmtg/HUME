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

;;; See docs/architecture.md's "Ref handling".
(define (git-diff/buffer-ref pane)
  (let ([entry (git-diff/buffer-entry pane)])
    (or (and entry (hash-ref entry "ref")) git-diff/ref)))

(define (git-diff/buffer-hunks pane)
  (let ([entry (git-diff/buffer-entry pane)])
    (if entry (hash-ref entry "hunks") '())))

;; ── Lifecycle ─────────────────────────────────────────────────────────────────

(register-hook! 'on-buffer-open
  (lambda (pane)
    (git-diff/init-buffer! pane git-diff/signs-default git-diff/inline-default)
    (git-diff/schedule-refresh! pane (git-diff/buffer-ref pane))))

(register-hook! 'on-buffer-enter
  (lambda (pane) (git-diff/schedule-branch-refresh! pane)))

;;; Drives the branch fetch — see docs/pipeline.md's "Branch tracking (`branch.scm`)".
(register-hook! 'on-option-change
  (lambda (key value)
    (when (equal? key "statusline")
      (git-diff/schedule-branch-refresh! (focused-pane)))))

(register-hook! 'on-text-changed
  (lambda (pane) (git-diff/schedule-refresh! pane (git-diff/buffer-ref pane))))

(register-hook! 'on-buffer-save
  (lambda (pane)
    (git-diff/cancel-fetch! pane)
    (git-diff/entry-set! pane "ref-text" #f)
    (git-diff/schedule-refresh! pane (git-diff/buffer-ref pane))
    (git-diff/schedule-branch-refresh! pane)))

(register-hook! 'on-buffer-close
  (lambda (pane)
    (git-diff/cancel-fetch! pane)
    (git-diff/cancel-branch-fetch! pane)
    (git-diff/remove-buffer! pane)))

;; ── Commands ──────────────────────────────────────────────────────────────────

(define-command! "git-diff/render-diff"
  "Draw hunks inline in a buffer: deleted lines as virtual lines, word highlights and a line tint. Arguments: pane, a decoration source name, a list of hunks in the shape `diff-buffer-lines` and `buffer-revision-diff` return. The source keeps the drawing apart from every other source's; an empty list clears it."
  git-diff/render-diff!)

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
                    (git-diff/entry-set! pane "ref-text" #f)
                    #t)
             (git-diff/toggle-flag! pane key))])
    (if enabled?
        (begin
          (git-diff/render-for! key pane (git-diff/buffer-hunks pane))
          (git-diff/force-refresh! pane (git-diff/buffer-ref pane)))
        (git-diff/render-for! key pane '()))
    (log! 'info (if enabled?
                    (string-append "git-diff: " label " on (" (git-diff/buffer-ref pane) ")")
                    (string-append "git-diff: " label " off")))))

(define-typed-command! "toggle-git-signs"
  "Toggle gutter +/~ signs and deletion boundary marks for the current buffer's git diff. Optional argument: a git ref to diff against, e.g. :toggle-git-signs HEAD~2 (default: the `ref` config value, shared with toggle-inline-diff)."
  (lambda (pane arg) (git-diff/run-toggle! pane "signs?" "signs" arg))
  #:complete "git-diff:refs")

(define-typed-command! "toggle-inline-diff"
  "Toggle inline git diff rendering (virtual deleted lines, word highlights, background tint). Optional argument: a git ref to diff against, e.g. :toggle-inline-diff HEAD~2 (default: the `ref` config value, shared with toggle-git-signs)."
  (lambda (pane arg) (git-diff/run-toggle! pane "inline?" "inline diff" arg))
  #:complete "git-diff:refs")
