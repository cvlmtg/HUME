;;; core:git-diff — diff.scm. See docs/pipeline.md. Word diff
;;; (`diff-words`) is called from render.scm, not here.

(require "state.scm")
(require "render.scm")

(provide git-diff/schedule-refresh! git-diff/force-refresh! git-diff/cancel-fetch!)

(define (git-diff/apply-hunks! pane hunks)
  (let ([entry (git-diff/buffer-entry pane)])
    (when (and entry (not (equal? (hash-ref entry "hunks") hunks)))
      (git-diff/entry-set! pane "hunks" hunks)
      (when (hash-ref entry "signs?") (git-diff/render-for! "signs?" pane hunks))
      (when (hash-ref entry "inline?") (git-diff/render-for! "inline?" pane hunks)))))

;;; `spawn-async!` callback for the `git show` below — see docs/pipeline.md
;;; for the exit-code/severity contract. `pane` may have closed while the
;;; job was in flight (`:bd` and job completion landing in the same frame —
;;; `on-buffer-close`'s `cancel-fetch!` can't help here, since the job was
;;; already moved off the cancellable slot and into this very callback);
;;; `entry-set!`/`cancel-job!` already no-op on a missing entry, but
;;; `diff-buffer-lines` is a `LivePane` builtin and would raise, so the
;;; success branch checks the entry first rather than calling it blind.
(define (git-diff/handle-fetch-result! pane stdout stderr exit-code)
  (git-diff/entry-set! pane "job" #f)
  (if (= exit-code 0)
      (when (git-diff/buffer-entry pane)
        (git-diff/entry-set! pane "ref-text" stdout)
        (git-diff/apply-hunks! pane (diff-buffer-lines pane stdout)))
      (begin
        (let ([entry (git-diff/buffer-entry pane)])
          (log! (cond [(= exit-code -1) 'error]
                      [(and entry (hash-ref entry "ref")) 'warn]
                      [else 'trace])
                (string-append "git-diff: `git show` failed: " (trim stderr))))
        (git-diff/entry-set! pane "ref-text" 'unavailable)
        (git-diff/apply-hunks! pane '()))))

;;; `git show <ref>:./<name>`, cwd = `path`'s directory.
(define (git-diff/fetch-ref! pane path ref)
  (git-diff/cancel-fetch! pane)
  (let ([job (spawn-async! "git"
                           (list "show" (string-append ref ":./" (file-name path)))
                           (parent-name path)
                           (lambda (stdout stderr exit-code)
                             (git-diff/handle-fetch-result! pane stdout stderr exit-code)))])
    (git-diff/entry-set! pane "job" job)))

;;; Immediate (non-debounced) refresh — `schedule-refresh!` is the debounced
;;; entry point every hook actually calls.
(define (git-diff/refresh! pane ref)
  (let ([entry (git-diff/buffer-entry pane)])
    (when (and entry (or (hash-ref entry "signs?") (hash-ref entry "inline?")))
      (let ([path (buffer-path pane)])
        (when path
          (let ([ref-text (hash-ref entry "ref-text")])
            (if (string? ref-text)
                (git-diff/apply-hunks! pane (diff-buffer-lines pane ref-text))
                (unless ref-text
                  (git-diff/fetch-ref! pane path ref)))))))))

;;; Forces a fetch even through a sticky `'unavailable` cache — see
;;; docs/pipeline.md for why, and why `hunks` is deliberately untouched.
(define (git-diff/force-refresh! pane ref)
  (let ([entry (git-diff/buffer-entry pane)])
    (when entry
      (unless (string? (hash-ref entry "ref-text"))
        (git-diff/entry-set! pane "ref-text" #f))
      (git-diff/refresh! pane ref))))

;;; Cancels any in-flight fetch for `pane` without firing its callback.
(define (git-diff/cancel-fetch! pane)
  (git-diff/cancel-job! pane "job"))

;;; `debounce-by`, keyed per buffer (not per pane — a command's own pane
;;; and a hook's pane-less value for the same buffer must still
;;; coalesce), at 150ms — see docs/pipeline.md.
(define git-diff/schedule-refresh!
  (debounce-by 150 git-diff/refresh! #:key (lambda (p . _) (buffer-key p))))
