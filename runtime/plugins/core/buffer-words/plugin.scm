;;; core:buffer-words

(unless (member "core:stdlib" (declared-plugins))
  (error "core:buffer-words: requires core:stdlib — (declare-plugin \"core:stdlib\") or (load-plugin \"core:stdlib\") before (load-plugin \"core:buffer-words\")"))

;; ── Config ────────────────────────────────────────────────────────────────────

(define bw/cfg (plugin-config))
(define bw/match (call! "stdlib/config-enum" "core:buffer-words" bw/cfg "match" 'string '(string fuzzy)))
(define bw/lines-per-tick (call! "stdlib/config-integer" "core:buffer-words" bw/cfg "lines" 200 1))

;;; Not 0 — see README.md's "Cursor-outward, line-windowed indexing".
(define bw/tick-delay-ms 16)

;; ── State ─────────────────────────────────────────────────────────────────────

(define bw/*buffers* (box (hash)))

;;; SSOT for a buffer's starting shape — see README.md's "Double-buffered cache".
(define (bw/fresh-entry)
  (hash "words" #f "building" #f "timer" #f "gen" 0))

(define (bw/entry bid)
  (let ([table (unbox bw/*buffers*)])
    (and (hash-contains? table bid) (hash-ref table bid))))

(define (bw/set! bid key value)
  (let ([entry (bw/entry bid)])
    (when entry
      (set-box! bw/*buffers*
                (hash-insert (unbox bw/*buffers*) bid (hash-insert entry key value))))))

;;; Cancels `entry`'s pending tick, if any — see README.md's "Cursor-
;;; outward, line-windowed indexing" for why `"gen"` is what actually makes
;;; staleness safe.
(define (bw/cancel-timer! entry)
  (let ([timer (hash-ref entry "timer")])
    (when timer (cancel-timer! timer))))

;;; Cancels any in-flight walk before installing a fresh entry.
(define (bw/init-buffer! bid)
  (bw/forget! bid)
  (set-box! bw/*buffers* (hash-insert (unbox bw/*buffers*) bid (bw/fresh-entry))))

(define (bw/forget! bid)
  (let ([entry (bw/entry bid)])
    (when entry (bw/cancel-timer! entry)))
  (set-box! bw/*buffers* (hash-remove (unbox bw/*buffers*) bid)))

;; ── Scanning ──────────────────────────────────────────────────────────────────

(define (bw/union-words set words)
  (if (null? words)
      set
      (bw/union-words (hashset-insert set (car words)) (cdr words))))

;;; 0-indexed cursor line, or the top of the buffer for a background bid.
(define (bw/anchor-line bid)
  (if (equal? bid (current-buffer))
      (let ([n (current-line-number)])
        (if n (- n 1) 0))
      0))

;;; One reindex tick — see README.md's "Cursor-outward, line-windowed
;;; indexing".
(define (bw/walk! bid gen fwd-line bwd-line)
  (let ([entry (bw/entry bid)])
    (when (and entry (= (hash-ref entry "gen") gen))
      (let* ([total (buffer-line-count bid)]
             [fwd-hi (min total (+ fwd-line bw/lines-per-tick))]
             [fwd-lines (if (< fwd-line fwd-hi) (buffer-lines bid #:start fwd-line #:end fwd-hi) '())]
             [bwd-hi (min bwd-line total)]
             [bwd-lo (max 0 (- bwd-hi bw/lines-per-tick))]
             [bwd-lines (if (< bwd-lo bwd-hi) (buffer-lines bid #:start bwd-lo #:end bwd-hi) '())]
             ;; Not a direct `(append fwd-lines bwd-lines)` — steel-core
             ;; drops entries past the 4th when the first arg is empty; see
             ;; runtime/plugins/core/git-diff/render.scm for the full
             ;; explanation.
             [scanned (if (null? fwd-lines) bwd-lines (append fwd-lines bwd-lines))]
             [found (apply append (map (lambda (l) (call! "stdlib/split-words" bid l)) scanned))]
             [building (or (hash-ref entry "building") (hashset))])
        (bw/set! bid "building" (bw/union-words building found))
        (if (and (>= fwd-hi total) (<= bwd-lo 0))
            (begin
              (bw/set! bid "words" (or (hash-ref (bw/entry bid) "building") (hashset)))
              (bw/set! bid "building" #f)
              (bw/set! bid "timer" #f))
            (bw/set! bid "timer"
                     (after bw/tick-delay-ms
                       (lambda () (bw/walk! bid gen fwd-hi bwd-lo)))))))))

;;; Cancels any in-flight walk and restarts it fresh, resurrecting `bid`'s
;;; entry first if missing — see README.md's "Double-buffered cache" and
;;; "Cursor-outward, line-windowed indexing".
(define (bw/reindex! bid)
  (unless (bw/entry bid)
    (when (member bid (buffers))
      (set-box! bw/*buffers* (hash-insert (unbox bw/*buffers*) bid (bw/fresh-entry)))))
  (let ([entry (bw/entry bid)])
    (when entry
      (bw/cancel-timer! entry)
      (let* ([anchor (bw/anchor-line bid)]
             [gen (+ (hash-ref entry "gen") 1)])
        (bw/set! bid "gen" gen)
        (bw/walk! bid gen anchor anchor)))))

;; ── Lifecycle ─────────────────────────────────────────────────────────────────

(register-hook! 'on-buffer-open
  (lambda (bid)
    (bw/init-buffer! bid)
    (bw/reindex! bid)))

(define bw/schedule-reindex! (debounce-by 150 bw/reindex!))

(register-hook! 'on-text-changed
  (lambda (bid) (bw/schedule-reindex! bid)))

(register-hook! 'on-buffer-close
  (lambda (bid) (bw/forget! bid)))

;;; Reindexes every open buffer on a global `word-chars` change — see
;;; README.md's "word-chars invalidation" for the buffer-scoped gap this
;;; doesn't close.
(register-hook! 'on-option-change
  (lambda (key value)
    (when (equal? key "word-chars")
      (for-each bw/reindex! (buffers)))))

;; ── Completion source ─────────────────────────────────────────────────────────

;;; Reads the cache `bw/walk!` builds — see README.md's "Double-buffered
;;; cache" and "Matching".
(define (bw/items bid prefix)
  (let* ([entry (bw/entry bid)]
         [words (and entry (or (hash-ref entry "words") (hash-ref entry "building")))])
    (if words
        (map (lambda (w) (hash "label" w))
             (filter (lambda (w) (not (equal? w prefix))) (hashset->list words)))
        '())))

(register-completion-source! "buffer-words"
  (lambda (id bid prefix)
    (completion-emit! id (bw/items bid prefix) #:incomplete #t))
  #:target 'buffer #:token 'word #:match bw/match)
