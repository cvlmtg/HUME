;;; core:git-diff — state.scm. See docs/architecture.md.

(provide git-diff/init-buffer! git-diff/remove-buffer!
         git-diff/buffer-entry git-diff/entry-set! git-diff/ensure-entry!
         git-diff/toggle-flag! git-diff/cancel-job!
         git-diff/add-cover! git-diff/remove-cover! git-diff/inline-covered?)

;;; Keyed by `(buffer-key pane)`, not `pane` itself — see docs/architecture.md's "State (`state.scm`)".
(define git-diff/*buffers* (box (hash)))

;;; SSOT for a buffer's starting shape.
(define (git-diff/fresh-entry signs? inline?)
  (hash "signs?" signs? "inline?" inline?
        "ref-text" #f "hunks" '() "job" #f "ref" #f "branch-job" #f
        "covered-by" '()))

(define (git-diff/buffer-entry pane)
  (let ([table (unbox git-diff/*buffers*)]
        [key (buffer-key pane)])
    (and (hash-contains? table key) (hash-ref table key))))

(define (git-diff/init-buffer! pane signs? inline?)
  (set-box! git-diff/*buffers*
            (hash-insert (unbox git-diff/*buffers*) (buffer-key pane)
                         (git-diff/fresh-entry signs? inline?))))

(define (git-diff/remove-buffer! pane)
  (set-box! git-diff/*buffers* (hash-remove (unbox git-diff/*buffers*) (buffer-key pane))))

;;; No-op when `pane`'s buffer has no tracked entry — see docs/architecture.md.
(define (git-diff/entry-set! pane key value)
  (let ([entry (git-diff/buffer-entry pane)])
    (when entry
      (set-box! git-diff/*buffers*
                (hash-insert (unbox git-diff/*buffers*) (buffer-key pane) (hash-insert entry key value))))))

;;; Unlike `entry-set!`, resurrects a missing entry rather than no-opping —
;;; see docs/architecture.md.
(define (git-diff/ensure-entry! pane)
  (unless (git-diff/buffer-entry pane)
    (set-box! git-diff/*buffers*
              (hash-insert (unbox git-diff/*buffers*) (buffer-key pane) (git-diff/fresh-entry #f #f)))))

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

(define (git-diff/inline-covered? pane)
  (let ([entry (git-diff/buffer-entry pane)])
    (and entry (not (null? (hash-ref entry "covered-by"))))))

;;; Shared by `diff.scm`'s and `branch.scm`'s cancel functions — see docs/architecture.md.
(define (git-diff/cancel-job! pane key)
  (let ([entry (git-diff/buffer-entry pane)])
    (when entry
      (let ([job (hash-ref entry key)])
        (when job (cancel-async! job)))
      (git-diff/entry-set! pane key #f))))
