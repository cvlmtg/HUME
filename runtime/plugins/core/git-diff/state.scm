;;; core:git-diff — state.scm. See docs/architecture.md.

(provide git-diff/init-buffer! git-diff/remove-buffer!
         git-diff/buffer-entry git-diff/entry-set! git-diff/ensure-entry!
         git-diff/buffer-ref git-diff/toggle-flag! git-diff/cancel-job! git-diff/spawn-job!
         git-diff/add-cover! git-diff/remove-cover!)

;;; Keyed by `(buffer-key pane)`, not `pane` itself — see docs/architecture.md's "State (`state.scm`)".
(define git-diff/*buffers* (box (hash)))

;;; SSOT for a buffer's starting shape.
(define (git-diff/fresh-entry signs? inline?)
  (hash "signs?" signs? "inline?" inline?
        "ref-text" #f "hunks" '() "signs-painted" '() "inline-painted" '()
        "job" #f "ref" #f "branch-job" #f "covered-by" '()))

(define (git-diff/buffer-entry pane)
  (hash-try-get (unbox git-diff/*buffers*) (buffer-key pane)))

(define (git-diff/put-entry! pane entry)
  (set-box! git-diff/*buffers*
            (hash-insert (unbox git-diff/*buffers*) (buffer-key pane) entry)))

(define (git-diff/init-buffer! pane signs? inline?)
  (git-diff/put-entry! pane (git-diff/fresh-entry signs? inline?)))

;;; Cancels the entry's in-flight jobs without firing their callbacks, then drops it.
(define (git-diff/remove-buffer! pane)
  (let ([entry (git-diff/buffer-entry pane)])
    (when entry
      (for-each (lambda (key)
                  (let ([job (hash-ref entry key)])
                    (when job (cancel-async! job))))
                '("job" "branch-job"))))
  (set-box! git-diff/*buffers* (hash-remove (unbox git-diff/*buffers*) (buffer-key pane))))

;;; No-op when `pane`'s buffer has no tracked entry — see docs/architecture.md.
(define (git-diff/entry-set! pane key value)
  (let ([entry (git-diff/buffer-entry pane)])
    (when entry
      (git-diff/put-entry! pane (hash-insert entry key value)))))

;;; Unlike `entry-set!`, resurrects a missing entry rather than no-opping —
;;; see docs/architecture.md.
(define (git-diff/ensure-entry! pane)
  (unless (git-diff/buffer-entry pane)
    (git-diff/put-entry! pane (git-diff/fresh-entry #f #f))))

;;; The buffer's own ref, else `default`.
(define (git-diff/buffer-ref pane default)
  (let ([entry (git-diff/buffer-entry pane)])
    (or (and entry (hash-ref entry "ref")) default)))

;;; Flips `key` (one of "signs?"/"inline?") and returns the new value.
(define (git-diff/toggle-flag! pane key)
  (git-diff/ensure-entry! pane)
  (let ([new? (not (hash-ref (git-diff/buffer-entry pane) key))])
    (git-diff/entry-set! pane key new?)
    new?))

;;; See docs/rendering.md's "Rendering another plugin's hunks".
(define (git-diff/add-cover! pane source)
  (git-diff/ensure-entry! pane)
  (let ([covers (hash-ref (git-diff/buffer-entry pane) "covered-by")])
    (unless (member source covers)
      (git-diff/entry-set! pane "covered-by" (cons source covers)))))

(define (git-diff/remove-cover! pane source)
  (let ([entry (git-diff/buffer-entry pane)])
    (when entry
      (git-diff/entry-set! pane "covered-by"
                           (filter (lambda (s) (not (equal? s source)))
                                   (hash-ref entry "covered-by"))))))

;;; Cancels the job in slot `key` without firing its callback — see docs/architecture.md.
(define (git-diff/cancel-job! pane key)
  (let ([entry (git-diff/buffer-entry pane)])
    (when entry
      (let ([job (hash-ref entry key)])
        (when job (cancel-async! job)))
      (git-diff/entry-set! pane key #f))))

;;; Starts `program` as the job in slot `key`, cancelling that slot's current
;;; job. `on-result` runs only while the job still owns the slot, so a result
;;; queued before a cancel or a replacement is dropped.
(define (git-diff/spawn-job! pane key program args cwd on-result)
  (git-diff/cancel-job! pane key)
  (let* ([id (box #f)]
         [job (spawn-async! program args
                (lambda (stdout stderr exit-code)
                  (let ([entry (git-diff/buffer-entry pane)])
                    (when (and entry (equal? (hash-ref entry key) (unbox id)))
                      (git-diff/entry-set! pane key #f)
                      (on-result stdout stderr exit-code))))
                #:cwd cwd)])
    (set-box! id job)
    (git-diff/entry-set! pane key job)))
