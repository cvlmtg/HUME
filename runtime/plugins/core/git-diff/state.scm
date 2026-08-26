;;; core:git-diff — state.scm (see README.md "File layout" and "How it
;;; works" → "State").

(provide git-diff/init-buffer! git-diff/remove-buffer!
         git-diff/buffer-entry git-diff/entry-set! git-diff/ensure-entry!
         git-diff/toggle-flag!)

(define git-diff/*buffers* (box (hash)))

;;; SSOT for a buffer's starting shape.
(define (git-diff/fresh-entry signs? inline?)
  (hash "signs?" signs? "inline?" inline?
        "ref-text" #f "hunks" '() "job" #f "ref" #f))

(define (git-diff/buffer-entry bid)
  (let ([table (unbox git-diff/*buffers*)])
    (and (hash-contains? table bid) (hash-ref table bid))))

(define (git-diff/init-buffer! bid signs? inline?)
  (set-box! git-diff/*buffers*
            (hash-insert (unbox git-diff/*buffers*) bid
                         (git-diff/fresh-entry signs? inline?))))

(define (git-diff/remove-buffer! bid)
  (set-box! git-diff/*buffers* (hash-remove (unbox git-diff/*buffers*) bid)))

;;; No-op when `bid` has no tracked entry — see README's "State" for why.
(define (git-diff/entry-set! bid key value)
  (let ([entry (git-diff/buffer-entry bid)])
    (when entry
      (set-box! git-diff/*buffers*
                (hash-insert (unbox git-diff/*buffers*) bid (hash-insert entry key value))))))

;;; Unlike `entry-set!`, resurrects a missing entry rather than no-opping —
;;; see README's "State" for why.
(define (git-diff/ensure-entry! bid)
  (unless (git-diff/buffer-entry bid)
    (set-box! git-diff/*buffers*
              (hash-insert (unbox git-diff/*buffers*) bid (git-diff/fresh-entry #f #f)))))

;;; Flips `key` (one of "signs?"/"inline?") and returns the new value.
(define (git-diff/toggle-flag! bid key)
  (git-diff/ensure-entry! bid)
  (let ([new? (not (hash-ref (git-diff/buffer-entry bid) key))])
    (git-diff/entry-set! bid key new?)
    new?))
