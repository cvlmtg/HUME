;;; core:git-diff — diff.scm. See docs/pipeline.md.

(require "state.scm")
(require "render.scm")

(provide git-diff/diffable-path git-diff/schedule-refresh! git-diff/refresh! git-diff/force-refresh!
         git-diff/invalidate-ref! git-diff/drop-ref-text! git-diff/hide-renderings!)

;;; Calls `paint!` with `target` when it differs from what `painted-key` holds in
;;; `entry`, and returns `entry` recording `target`.
(define (git-diff/reconcile-rendering entry pane painted-key target paint!)
  (if (equal? (hash-ref entry painted-key) target)
      entry
      (begin (paint! pane target)
             (hash-insert entry painted-key target))))

(define (git-diff/signs-visible? entry)
  (hash-ref entry "signs?"))

(define (git-diff/inline-visible? entry)
  (and (hash-ref entry "inline?") (null? (hash-ref entry "covered-by"))))

;;; The file `pane`'s buffer diffs, or #f when it has none or `entry` shows no rendering.
(define (git-diff/diffable-path pane entry)
  (and entry
       (or (git-diff/signs-visible? entry) (git-diff/inline-visible? entry))
       (buffer-path pane)))

(define (git-diff/reconcile-signs entry pane signs)
  (git-diff/reconcile-rendering entry pane "signs-painted" signs git-diff/render-signs!))

(define (git-diff/reconcile-inline entry pane target)
  (git-diff/reconcile-rendering entry pane "inline-painted" target
    (lambda (p hunks) (git-diff/render-diff! p git-diff/*source* hunks))))

;;; Paints `hunks` in each visible rendering of `entry`, `pane`'s current entry, clears
;;; each hidden one, and stores the entry.
(define (git-diff/apply-hunks! pane entry hunks)
  (git-diff/put-entry! pane
    (git-diff/reconcile-inline
      (git-diff/reconcile-signs entry pane
        (if (git-diff/signs-visible? entry) (git-diff/hunks->signs hunks) '()))
      pane (if (git-diff/inline-visible? entry) hunks '()))))

;;; Clears each hidden rendering and leaves the visible ones as painted.
(define (git-diff/hide-renderings! pane)
  (let* ([entry (git-diff/buffer-entry pane)]
         [entry (if (git-diff/signs-visible? entry)
                    entry
                    (git-diff/reconcile-signs entry pane '()))]
         [entry (if (git-diff/inline-visible? entry)
                    entry
                    (git-diff/reconcile-inline entry pane '()))])
    (git-diff/put-entry! pane entry)))

;;; `spawn-async!` callback for `refresh!`'s `git cat-file`; a failure logs at
;;; `failure-severity` — see docs/pipeline.md's "Severity routing".
(define (git-diff/handle-fetch-result! pane failure-severity result)
  (let ([exit-code (hash-ref result 'exit)]
        [stdout (hash-ref result 'stdout)])
    (if (equal? exit-code 0)
        (git-diff/apply-hunks! pane
                               (hash-insert (git-diff/buffer-entry pane) "ref-text" stdout)
                               (hash-ref (diff-buffer-lines pane stdout) 'hunks))
        (begin
          (log! (if exit-code failure-severity 'error)
                (string-append "git-diff: `git cat-file` failed: "
                               (trim (hash-ref result 'stderr))))
          (git-diff/apply-hunks! pane
                                 (hash-insert (git-diff/buffer-entry pane) "ref-text" 'unavailable)
                                 '())))))

;;; Immediate (non-debounced) refresh — `schedule-refresh!` is the debounced entry point.
;;; A buffer without its own ref diffs against `default-ref`.
(define (git-diff/refresh! pane default-ref failure-severity)
  (let* ([entry (git-diff/buffer-entry pane)]
         [path (git-diff/diffable-path pane entry)])
    (when path
      (unless (equal? (hash-ref entry "ref-path") path)
        (git-diff/drop-ref-text! pane))
      (let ([ref-text (git-diff/entry-get pane "ref-text")])
        (cond
          [(string? ref-text)
           (git-diff/apply-hunks! pane (git-diff/buffer-entry pane)
                                  (hash-ref (diff-buffer-lines pane ref-text) 'hunks))]
          [(and (not ref-text) (not (git-diff/slot-job pane "diff")))
           (git-diff/entry-set! pane "ref-path" path)
           (git-diff/spawn-job! pane "diff" "git"
                                (list "cat-file" "--filters" "--end-of-options"
                                      (string-append (git-diff/buffer-ref pane default-ref)
                                                     ":./" (file-name path)))
                                (parent-name path)
                                (lambda (result)
                                  (git-diff/handle-fetch-result! pane failure-severity result)))])))))

;;; Forces a fetch even through a sticky `'unavailable` cache — see docs/pipeline.md.
(define (git-diff/force-refresh! pane default-ref failure-severity)
  (unless (string? (git-diff/entry-get pane "ref-text"))
    (git-diff/entry-set! pane "ref-text" #f))
  (git-diff/refresh! pane default-ref failure-severity))

;;; Drops the cached blob and cancels any fetch in flight without firing its callback.
(define (git-diff/drop-ref-text! pane)
  (git-diff/cancel-job! pane "diff")
  (git-diff/entry-set! pane "ref-text" #f))

;;; Drops the cached blob and any fetch in flight, then schedules a refetch.
(define (git-diff/invalidate-ref! pane default-ref)
  (git-diff/drop-ref-text! pane)
  (git-diff/schedule-refresh! pane default-ref))

;;; `debounce-by`, keyed per buffer, at 150ms — see docs/pipeline.md.
(define git-diff/schedule-refresh!
  (debounce-by 150 (lambda (pane default-ref) (git-diff/refresh! pane default-ref 'trace))
               #:key (lambda (p . _) (buffer-key p))))
