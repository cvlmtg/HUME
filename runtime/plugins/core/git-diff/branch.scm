;;; core:git-diff — branch.scm. See docs/pipeline.md.

(require "state.scm")

(provide git-diff/schedule-branch-refresh! git-diff/cancel-branch-fetch!)

;;; `spawn-async!` callback for `git rev-parse --abbrev-ref HEAD` — see
;;; docs/pipeline.md for the liveness-check and severity choice.
(define (git-diff/handle-branch-result! bid stdout stderr exit-code)
  (git-diff/entry-set! bid "branch-job" #f)
  (when (git-diff/buffer-entry bid)
    (if (= exit-code 0)
        (set-statusline-text! "git-branch" bid (string-append "(" (trim stdout) ")"))
        (begin
          (when (= exit-code -1)
            (log! 'error (string-append "git-diff: `git rev-parse` failed: " (trim stderr))))
          (set-statusline-text! "git-branch" bid "")))))

;;; `git rev-parse --abbrev-ref HEAD`, cwd = `path`'s directory.
(define (git-diff/fetch-branch! bid path)
  (git-diff/cancel-branch-fetch! bid)
  (let ([job (spawn-async! "git" '("rev-parse" "--abbrev-ref" "HEAD") (parent-name path)
                           (lambda (stdout stderr exit-code)
                             (git-diff/handle-branch-result! bid stdout stderr exit-code)))])
    (git-diff/entry-set! bid "branch-job" job)))

;;; Gates the fetch on `"steel:git-branch"` being placed — see
;;; docs/pipeline.md for why.
(define (git-diff/branch-element-placed?)
  (string-contains? (get-option "statusline") "steel:git-branch"))

;;; Immediate (non-debounced) — re-reads the buffer's live entry and path,
;;; since `buffer-path`, unlike `entry-set!`, hard-errors on a dead bid.
(define (git-diff/refresh-branch! bid)
  (when (and (git-diff/buffer-entry bid) (git-diff/branch-element-placed?))
    (let ([path (buffer-path bid)])
      (when path (git-diff/fetch-branch! bid path)))))

;;; Cancels any in-flight branch fetch for `bid` without firing its callback.
(define (git-diff/cancel-branch-fetch! bid)
  (git-diff/cancel-job! bid "branch-job"))

;;; `debounce-by`, keyed per `bid`, at 150ms — see docs/pipeline.md.
(define git-diff/schedule-branch-refresh! (debounce-by 150 git-diff/refresh-branch!))
