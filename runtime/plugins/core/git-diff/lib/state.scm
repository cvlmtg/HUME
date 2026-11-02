;;; core:git-diff — state.scm. See docs/architecture.md.

(provide git-diff/remove-buffer!
         git-diff/buffer-entry git-diff/entry-get git-diff/entry-set! git-diff/ensure-entry!
         git-diff/buffer-ref git-diff/toggle-flag! git-diff/slot-job git-diff/cancel-job! git-diff/spawn-job!
         git-diff/add-cover! git-diff/remove-cover!)

;;; Keyed by `(buffer-key pane)`, not `pane` itself — see docs/architecture.md's "State (`state.scm`)".
(define git-diff/*buffers* (box (hash)))

;;; SSOT for a buffer's starting shape.
(define (git-diff/fresh-entry signs? inline?)
  (hash "signs?" signs? "inline?" inline?
        "ref-text" #f "signs-painted" '() "inline-painted" '()
        "jobs" (hash) "ref" #f "covered-by" '()))

(define (git-diff/buffer-entry pane)
  (hash-try-get (unbox git-diff/*buffers*) (buffer-key pane)))

;;; `key`'s value in `pane`'s entry, or #f when the buffer has no tracked entry.
(define (git-diff/entry-get pane key)
  (let ([entry (git-diff/buffer-entry pane)])
    (and entry (hash-ref entry key))))

(define (git-diff/put-entry! pane entry)
  (set-box! git-diff/*buffers*
            (hash-insert (unbox git-diff/*buffers*) (buffer-key pane) entry)))

;;; Cancels the entry's in-flight jobs without firing their callbacks, then drops it.
(define (git-diff/remove-buffer! pane)
  (let ([jobs (git-diff/entry-get pane "jobs")])
    (when jobs (for-each cancel-async! (hash-values->list jobs))))
  (set-box! git-diff/*buffers* (hash-remove (unbox git-diff/*buffers*) (buffer-key pane))))

;;; Raises when `pane`'s buffer has no entry — see docs/architecture.md.
(define (git-diff/entry-set! pane key value)
  (let ([entry (git-diff/buffer-entry pane)])
    (unless entry
      (error (string-append "core:git-diff: no state for the buffer to set \"" key "\" in")))
    (git-diff/put-entry! pane (hash-insert entry key value))))

;;; Creates `pane`'s entry with these flags when it has none; #t when it did.
(define (git-diff/ensure-entry! pane signs? inline?)
  (and (not (git-diff/buffer-entry pane))
       (begin (git-diff/put-entry! pane (git-diff/fresh-entry signs? inline?)) #t)))

;;; The buffer's own ref, else `default`.
(define (git-diff/buffer-ref pane default)
  (or (git-diff/entry-get pane "ref") default))

;;; Flips `key` (one of "signs?"/"inline?") and returns the new value.
(define (git-diff/toggle-flag! pane key)
  (let ([new? (not (git-diff/entry-get pane key))])
    (git-diff/entry-set! pane key new?)
    new?))

;;; See docs/rendering.md's "Rendering another plugin's hunks".
(define (git-diff/add-cover! pane source)
  (let ([covers (git-diff/entry-get pane "covered-by")])
    (unless (member source covers)
      (git-diff/entry-set! pane "covered-by" (cons source covers)))))

(define (git-diff/remove-cover! pane source)
  (git-diff/entry-set! pane "covered-by"
                       (filter (lambda (s) (not (equal? s source)))
                               (git-diff/entry-get pane "covered-by"))))

;;; The job in slot `key`, or #f.
(define (git-diff/slot-job pane key)
  (let ([jobs (git-diff/entry-get pane "jobs")])
    (and jobs (hash-try-get jobs key))))

;;; Puts `job` in slot `key`, or empties the slot when `job` is #f.
(define (git-diff/set-slot! pane key job)
  (let ([jobs (git-diff/entry-get pane "jobs")])
    (git-diff/entry-set! pane "jobs" (if job (hash-insert jobs key job) (hash-remove jobs key)))))

;;; Cancels the job in slot `key` without firing its callback — see docs/architecture.md.
(define (git-diff/cancel-job! pane key)
  (let ([job (git-diff/slot-job pane key)])
    (when job
      (cancel-async! job)
      (git-diff/set-slot! pane key #f))))

;;; Starts `program` as the job in slot `key`, cancelling that slot's current
;;; job. `on-result` runs only while the job still owns the slot, so a result
;;; queued before a cancel or a replacement is dropped.
(define (git-diff/spawn-job! pane key program args cwd on-result)
  (git-diff/cancel-job! pane key)
  (let* ([id (box #f)]
         [job (spawn-async! program args
                (lambda (result)
                  (when (equal? (git-diff/slot-job pane key) (unbox id))
                    (git-diff/set-slot! pane key #f)
                    (on-result result)))
                #:cwd cwd)])
    (set-box! id job)
    (git-diff/set-slot! pane key job)))
