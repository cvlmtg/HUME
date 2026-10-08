;;; core:git-diff — diff.scm. See docs/pipeline.md.

(require "state.scm")
(require "render.scm")

(provide git-diff/schedule-refresh! git-diff/refresh! git-diff/force-refresh!
         git-diff/invalidate-ref! git-diff/drop-ref-text! git-diff/hide-renderings!)

;;; Calls `paint!` with `target` when it differs from what `painted-key` last recorded.
(define (git-diff/reconcile-rendering! entry pane painted-key target paint!)
  (unless (equal? (hash-ref entry painted-key) target)
    (paint! pane target)
    (git-diff/entry-set! pane painted-key target)))

(define (git-diff/signs-visible? entry)
  (hash-ref entry "signs?"))

(define (git-diff/inline-visible? entry)
  (and (hash-ref entry "inline?") (null? (hash-ref entry "covered-by"))))

(define (git-diff/reconcile-signs! entry pane target)
  (git-diff/reconcile-rendering! entry pane "signs-painted" target git-diff/render-signs!))

(define (git-diff/reconcile-inline! entry pane target)
  (git-diff/reconcile-rendering! entry pane "inline-painted" target
    (lambda (p hunks) (git-diff/render-diff! p git-diff/*source* hunks))))

;;; Paints `hunks` in each visible rendering and clears each hidden one.
(define (git-diff/apply-hunks! pane hunks)
  (let ([entry (git-diff/buffer-entry pane)])
    (when entry
      (git-diff/reconcile-signs! entry pane (if (git-diff/signs-visible? entry) hunks '()))
      (git-diff/reconcile-inline! entry pane (if (git-diff/inline-visible? entry) hunks '())))))

;;; Clears each hidden rendering and leaves the visible ones as painted.
(define (git-diff/hide-renderings! pane)
  (let ([entry (git-diff/buffer-entry pane)])
    (when entry
      (unless (git-diff/signs-visible? entry)
        (git-diff/reconcile-signs! entry pane '()))
      (unless (git-diff/inline-visible? entry)
        (git-diff/reconcile-inline! entry pane '())))))

;;; `spawn-async!` callback for the `git cat-file` below — see docs/pipeline.md.
(define (git-diff/handle-fetch-result! pane stdout stderr exit-code)
  (if (= exit-code 0)
      (begin
        (git-diff/entry-set! pane "ref-text" stdout)
        (git-diff/apply-hunks! pane (diff-buffer-lines pane stdout)))
      (begin
        (let ([entry (git-diff/buffer-entry pane)])
          (log! (cond [(= exit-code -1) 'error]
                      [(hash-ref entry "ref") 'warn]
                      [else 'trace])
                (string-append "git-diff: `git cat-file` failed: " (trim stderr))))
        (git-diff/entry-set! pane "ref-text" 'unavailable)
        (git-diff/apply-hunks! pane '()))))

;;; `git cat-file --filters <ref>:./<name>`, cwd = `path`'s directory.
(define (git-diff/fetch-ref! pane path ref)
  (git-diff/spawn-job! pane "job" "git"
                       (list "cat-file" "--filters" "--end-of-options"
                             (string-append ref ":./" (file-name path)))
                       (parent-name path)
                       (lambda (stdout stderr exit-code)
                         (git-diff/handle-fetch-result! pane stdout stderr exit-code))))

;;; Immediate (non-debounced) refresh — `schedule-refresh!` is the debounced entry point.
;;; A buffer without its own ref diffs against `default-ref`.
(define (git-diff/refresh! pane default-ref)
  (let ([entry (git-diff/buffer-entry pane)])
    (when (and entry (or (hash-ref entry "signs?") (hash-ref entry "inline?")))
      (let ([path (buffer-path pane)])
        (when path
          (let ([ref-text (hash-ref entry "ref-text")])
            (cond
              [(string? ref-text)
               (git-diff/apply-hunks! pane (diff-buffer-lines pane ref-text))]
              [(and (not ref-text) (not (hash-ref entry "job")))
               (git-diff/fetch-ref! pane path (git-diff/buffer-ref pane default-ref))])))))))

;;; Forces a fetch even through a sticky `'unavailable` cache — see docs/pipeline.md.
(define (git-diff/force-refresh! pane default-ref)
  (let ([entry (git-diff/buffer-entry pane)])
    (when entry
      (unless (string? (hash-ref entry "ref-text"))
        (git-diff/entry-set! pane "ref-text" #f))
      (git-diff/refresh! pane default-ref))))

;;; Drops the cached blob and cancels any fetch in flight without firing its callback.
(define (git-diff/drop-ref-text! pane)
  (git-diff/cancel-job! pane "job")
  (git-diff/entry-set! pane "ref-text" #f))

;;; Drops the cached blob and any fetch in flight, then schedules a refetch.
(define (git-diff/invalidate-ref! pane default-ref)
  (git-diff/drop-ref-text! pane)
  (git-diff/schedule-refresh! pane default-ref))

;;; `debounce-by`, keyed per buffer, at 150ms — see docs/pipeline.md.
(define git-diff/schedule-refresh!
  (debounce-by 150 git-diff/refresh! #:key (lambda (p . _) (buffer-key p))))
