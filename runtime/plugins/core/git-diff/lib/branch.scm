;;; core:git-diff — branch.scm. See docs/pipeline.md.

(require "state.scm")

(provide git-diff/schedule-branch-refresh!)

;;; The element's text for branch `name`: `(name)` for `'parens`, `⎇ name` for `'icon`.
(define (git-diff/format-branch style name)
  (if (equal? style 'icon)
      (string-append "⎇ " name)
      (string-append "(" name ")")))

;;; `spawn-async!` callback for `git rev-parse --abbrev-ref HEAD` — see docs/pipeline.md.
(define (git-diff/handle-branch-result! pane style stdout stderr exit-code)
  (if (equal? exit-code 0)
      (set-statusline-text! "git-branch" pane (git-diff/format-branch style (trim stdout)))
      (begin
        (when (not exit-code)
          (log! 'error (string-append "git-diff: `git rev-parse` failed: " (trim stderr))))
        (set-statusline-text! "git-branch" pane ""))))

;;; `git rev-parse --abbrev-ref HEAD`, cwd = `path`'s directory.
(define (git-diff/fetch-branch! pane style path)
  (git-diff/spawn-job! pane "branch-job" "git" '("rev-parse" "--abbrev-ref" "HEAD")
                       (parent-name path)
                       (lambda (stdout stderr exit-code)
                         (git-diff/handle-branch-result! pane style stdout stderr exit-code))))

;;; Gates the fetch on `"steel:git-branch"` being placed — see docs/pipeline.md.
(define (git-diff/branch-element-placed?)
  (string-contains? (get-option "statusline") "steel:git-branch"))

;;; Immediate (non-debounced) — see docs/pipeline.md's "Branch tracking (`branch.scm`)".
(define (git-diff/refresh-branch! pane style)
  (when (and (git-diff/buffer-entry pane) (git-diff/branch-element-placed?))
    (let ([path (buffer-path pane)])
      (when path (git-diff/fetch-branch! pane style path)))))

;;; `debounce-by`, keyed per buffer, at 150ms — see docs/pipeline.md.
(define git-diff/schedule-branch-refresh!
  (debounce-by 150 git-diff/refresh-branch! #:key (lambda (p . _) (buffer-key p))))
