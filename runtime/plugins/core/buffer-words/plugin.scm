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

(define (bw/install! bid entry)
  (set-box! bw/*buffers* (hash-insert (unbox bw/*buffers*) bid entry)))

;;; Cancels `entry`'s pending tick, if any — see README.md's "Cursor-
;;; outward, line-windowed indexing" for why `"gen"` is what actually makes
;;; staleness safe.
(define (bw/cancel-timer! entry)
  (let ([timer (hash-ref entry "timer")])
    (when timer (cancel-timer! timer))))

(define (bw/forget! bid)
  (let ([entry (bw/entry bid)])
    (when entry (bw/cancel-timer! entry)))
  (set-box! bw/*buffers* (hash-remove (unbox bw/*buffers*) bid)))

;; ── Scanning ──────────────────────────────────────────────────────────────────

;;; Folds every word `(split-words line wc)` finds across `lines` into `set`.
(define (bw/add-lines set lines wc)
  (foldl (lambda (line words)
           (foldl (lambda (w s) (hashset-insert s w)) words (split-words line wc)))
         set lines))

;;; 0-indexed cursor line, or the top of the buffer for a background bid.
(define (bw/anchor-line bid)
  (if (equal? bid (current-buffer))
      (let ([n (current-line-number)])
        (if n (- n 1) 0))
      0))

;;; See README.md's "Double-buffered cache".
(define (bw/finish-entry entry building)
  (hash-insert
    (hash-insert
      (hash-insert entry "words" (map (lambda (w) (hash "label" w)) (hashset->list building)))
      "building" #f)
    "timer" #f))

(define (bw/continue-entry entry building timer)
  (hash-insert (hash-insert entry "building" building) "timer" timer))

;;; One reindex tick — see README.md's "Cursor-outward, line-windowed
;;; indexing".
(define (bw/walk! bid gen wc fwd-line bwd-line)
  (let ([entry (bw/entry bid)])
    (when (and entry (= (hash-ref entry "gen") gen))
      (let* ([total (buffer-line-count bid)]
             [fwd-hi (min total (+ fwd-line bw/lines-per-tick))]
             [fwd-lines (if (< fwd-line fwd-hi) (buffer-lines bid #:start fwd-line #:end fwd-hi) '())]
             [bwd-hi (min bwd-line total)]
             [bwd-lo (max 0 (- bwd-hi bw/lines-per-tick))]
             [bwd-lines (if (< bwd-lo bwd-hi) (buffer-lines bid #:start bwd-lo #:end bwd-hi) '())]
             [building (bw/add-lines (bw/add-lines (or (hash-ref entry "building") (hashset)) fwd-lines wc)
                                      bwd-lines wc)])
        (bw/install! bid
          (if (and (>= fwd-hi total) (<= bwd-lo 0))
              (bw/finish-entry entry building)
              (bw/continue-entry entry building
                                  (after bw/tick-delay-ms
                                    (lambda () (bw/walk! bid gen wc fwd-hi bwd-lo))))))))))

;;; Cancels any in-flight walk and restarts it fresh, resurrecting `bid`'s
;;; entry first if missing — see README.md's "Double-buffered cache" and
;;; "Cursor-outward, line-windowed indexing".
(define (bw/reindex! bid)
  (unless (bw/entry bid)
    (when (member bid (buffers))
      (bw/install! bid (bw/fresh-entry))))
  (let ([entry (bw/entry bid)])
    (when entry
      (bw/cancel-timer! entry)
      (let* ([anchor (bw/anchor-line bid)]
             [gen (+ (hash-ref entry "gen") 1)]
             [wc (get-option bid "word-chars")])
        (bw/install! bid (hash-insert entry "gen" gen))
        (bw/walk! bid gen wc anchor anchor)))))

;; ── Lifecycle ─────────────────────────────────────────────────────────────────

(register-hook! 'on-buffer-open
  (lambda (bid)
    (bw/forget! bid)
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
  (let ([entry (bw/entry bid)])
    (if (not entry)
        '()
        (let ([ready (hash-ref entry "words")])
          (if ready
              (filter (lambda (it) (not (equal? (hash-ref it "label") prefix))) ready)
              (let ([words (hash-ref entry "building")])
                (if words
                    (map (lambda (w) (hash "label" w))
                         (filter (lambda (w) (not (equal? w prefix))) (hashset->list words)))
                    '())))))))

(register-completion-source! "buffer-words"
  (lambda (id bid prefix)
    (completion-emit! id (bw/items bid prefix) #:incomplete #t))
  #:target 'buffer #:token 'word #:match bw/match)
