;;; core:git-diff — branch.scm. See docs/pipeline.md.

(require "state.scm")

(provide git-diff/schedule-branch-refresh! git-diff/cancel-branch-fetch!)

;;; `spawn-async!` callback for `git rev-parse --abbrev-ref HEAD` — see docs/pipeline.md.
(define (git-diff/handle-branch-result! pane stdout stderr exit-code)
  (if (= exit-code 0)
      (set-statusline-text! "git-branch" pane (string-append "(" (trim stdout) ")"))
      (begin
        (when (= exit-code -1)
          (log! 'error (string-append "git-diff: `git rev-parse` failed: " (trim stderr))))
        (set-statusline-text! "git-branch" pane ""))))

;;; `git rev-parse --abbrev-ref HEAD`, cwd = `path`'s directory.
(define (git-diff/fetch-branch! pane path)
  (git-diff/spawn-job! pane "branch-job" "git" '("rev-parse" "--abbrev-ref" "HEAD")
                       (parent-name path)
                       (lambda (stdout stderr exit-code)
                         (git-diff/handle-branch-result! pane stdout stderr exit-code))))

;;; Gates the fetch on `"steel:git-branch"` being placed — see docs/pipeline.md.
(define (git-diff/branch-element-placed?)
  (string-contains? (get-option "statusline") "steel:git-branch"))

;;; Immediate (non-debounced) — see docs/pipeline.md's "Branch tracking (`branch.scm`)".
(define (git-diff/refresh-branch! pane)
  (when (and (git-diff/buffer-entry pane) (git-diff/branch-element-placed?))
    (let ([path (buffer-path pane)])
      (when path (git-diff/fetch-branch! pane path)))))

;;; Cancels any in-flight branch fetch for `pane` without firing its callback.
(define (git-diff/cancel-branch-fetch! pane)
  (git-diff/cancel-job! pane "branch-job"))

;;; `debounce-by`, keyed per buffer, at 150ms — see docs/pipeline.md.
(define git-diff/schedule-branch-refresh!
  (debounce-by 150 git-diff/refresh-branch! #:key (lambda (p . _) (buffer-key p))))
