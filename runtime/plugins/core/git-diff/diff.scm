;;; core:git-diff — diff.scm. See docs/pipeline.md. Word diff
;;; (`diff-words`) is called from render.scm, not here.

(require "state.scm")
(require "render.scm")

(provide git-diff/schedule-refresh! git-diff/force-refresh! git-diff/cancel-fetch!)

(define (git-diff/apply-hunks! bid hunks)
  (let ([entry (git-diff/buffer-entry bid)])
    (when (and entry (not (equal? (hash-ref entry "hunks") hunks)))
      (git-diff/entry-set! bid "hunks" hunks)
      (when (hash-ref entry "signs?") (git-diff/render-for! "signs?" bid hunks))
      (when (hash-ref entry "inline?") (git-diff/render-for! "inline?" bid hunks)))))

;;; `spawn-async!` callback for the `git show` below — see docs/pipeline.md
;;; for the exit-code/severity contract.
(define (git-diff/handle-fetch-result! bid stdout stderr exit-code)
  (git-diff/entry-set! bid "job" #f)
  (if (= exit-code 0)
      (begin
        (git-diff/entry-set! bid "ref-text" stdout)
        (git-diff/apply-hunks! bid (diff-buffer-lines bid stdout)))
      (begin
        (let ([entry (git-diff/buffer-entry bid)])
          (log! (cond [(= exit-code -1) 'error]
                      [(and entry (hash-ref entry "ref")) 'warn]
                      [else 'trace])
                (string-append "git-diff: `git show` failed: " (trim stderr))))
        (git-diff/entry-set! bid "ref-text" 'unavailable)
        (git-diff/apply-hunks! bid '()))))

;;; `git show <ref>:./<name>`, cwd = `path`'s directory.
(define (git-diff/fetch-ref! bid path ref)
  (git-diff/cancel-fetch! bid)
  (let ([job (spawn-async! "git"
                           (list "show" (string-append ref ":./" (file-name path)))
                           (parent-name path)
                           (lambda (stdout stderr exit-code)
                             (git-diff/handle-fetch-result! bid stdout stderr exit-code)))])
    (git-diff/entry-set! bid "job" job)))

;;; Immediate (non-debounced) refresh — `schedule-refresh!` is the debounced
;;; entry point every hook actually calls.
(define (git-diff/refresh! bid ref)
  (let ([entry (git-diff/buffer-entry bid)])
    (when (and entry (or (hash-ref entry "signs?") (hash-ref entry "inline?")))
      (let ([path (buffer-path bid)])
        (when path
          (let ([ref-text (hash-ref entry "ref-text")])
            (if (string? ref-text)
                (git-diff/apply-hunks! bid (diff-buffer-lines bid ref-text))
                (unless ref-text
                  (git-diff/fetch-ref! bid path ref)))))))))

;;; Forces a fetch even through a sticky `'unavailable` cache — see
;;; docs/pipeline.md for why, and why `hunks` is deliberately untouched.
(define (git-diff/force-refresh! bid ref)
  (let ([entry (git-diff/buffer-entry bid)])
    (when entry
      (unless (string? (hash-ref entry "ref-text"))
        (git-diff/entry-set! bid "ref-text" #f))
      (git-diff/refresh! bid ref))))

;;; Cancels any in-flight fetch for `bid` without firing its callback.
(define (git-diff/cancel-fetch! bid)
  (git-diff/cancel-job! bid "job"))

;;; `debounce-by`, keyed per `bid`, at 150ms — see docs/pipeline.md.
(define git-diff/schedule-refresh! (debounce-by 150 git-diff/refresh!))
