;;; core:markdown — state.scm. See docs/architecture.md.

(provide md-preview/entry md-preview/track-buffer! md-preview/remove-buffer!
         md-preview/note-tree! md-preview/toggle-enabled! md-preview/warn-once!
         md-preview/box-insert! md-preview/lists-generation md-preview/note-lists!)

(define md-preview/*buffers* (box (hash)))

(define md-preview/*warned* (box '()))

(define (md-preview/box-insert! table-box key value)
  (let ([table (unbox table-box)])
    (set-box! table-box #f)
    (set-box! table-box (hash-insert table key value))))

(define (md-preview/entry pane)
  (hash-try-get (unbox md-preview/*buffers*) (buffer-key pane)))

(define (md-preview/put-entry! pane entry)
  (md-preview/box-insert! md-preview/*buffers* (buffer-key pane) entry))

(define (md-preview/track-buffer! pane enabled?)
  (unless (md-preview/entry pane)
    (md-preview/put-entry! pane (hash 'enabled? enabled? 'generation #f 'lists-generation #f))))

(define (md-preview/remove-buffer! pane)
  (set-box! md-preview/*buffers*
            (hash-remove (unbox md-preview/*buffers*) (buffer-key pane))))

(define (md-preview/entry-set! pane key value)
  (md-preview/put-entry! pane (hash-insert (md-preview/entry pane) key value)))

(define (md-preview/note-tree! pane)
  (md-preview/entry-set! pane 'generation (buffer-generation pane)))

(define (md-preview/lists-generation pane)
  (hash-ref (md-preview/entry pane) 'lists-generation))

(define (md-preview/note-lists! pane generation)
  (md-preview/entry-set! pane 'lists-generation generation))

(define (md-preview/toggle-enabled! pane)
  (let ([on? (not (hash-ref (md-preview/entry pane) 'enabled?))])
    (md-preview/entry-set! pane 'enabled? on?)
    on?))

(define (md-preview/warn-once! name message)
  (unless (member name (unbox md-preview/*warned*))
    (set-box! md-preview/*warned* (cons name (unbox md-preview/*warned*)))
    (log! 'warning message)))
