;;; core:buffer-words

(unless (member "core:stdlib" (declared-plugins))
  (error "core:buffer-words: requires core:stdlib — (declare-plugin \"core:stdlib\") or (load-plugin \"core:stdlib\") before (load-plugin \"core:buffer-words\")"))

;; ── Config ────────────────────────────────────────────────────────────────────

(define bw/cfg (plugin-config))
(define bw/match (call! "stdlib/config-enum" "core:buffer-words" bw/cfg "match" 'string '(string fuzzy)))
(define bw/lines-per-tick (call! "stdlib/config-integer" "core:buffer-words" bw/cfg "lines" 200 1))

;; ── State ─────────────────────────────────────────────────────────────────────
;; bid -> (hash "words" <hashset-or-#f> "building" <hashset-or-#f> "timer" <id-or-#f>)
;; See README.md's "Double-buffered cache".

(define bw/*buffers* (box (hash)))

(define (bw/entry bid)
  (let ([table (unbox bw/*buffers*)])
    (and (hash-contains? table bid) (hash-ref table bid))))

(define (bw/set! bid key value)
  (let ([entry (bw/entry bid)])
    (when entry
      (set-box! bw/*buffers*
                (hash-insert (unbox bw/*buffers*) bid (hash-insert entry key value))))))

;;; Cancels `entry`'s in-flight walk, if any — the one pending tick, since
;;; each tick is what schedules the next.
(define (bw/cancel-timer! entry)
  (let ([timer (hash-ref entry "timer")])
    (when timer (cancel-timer! timer))))

;;; Cancels any in-flight walk before installing a fresh entry — see
;;; README.md's "Cursor-outward, line-windowed indexing".
(define (bw/init-buffer! bid)
  (bw/forget! bid)
  (set-box! bw/*buffers*
            (hash-insert (unbox bw/*buffers*) bid
                         (hash "words" #f "building" #f "timer" #f))))

(define (bw/forget! bid)
  (let ([entry (bw/entry bid)])
    (when entry (bw/cancel-timer! entry)))
  (set-box! bw/*buffers* (hash-remove (unbox bw/*buffers*) bid)))

;; ── Word classification ───────────────────────────────────────────────────────

;;; Word-char approximation — see README.md's "Non-ASCII words".
(define (bw/word-char? extra ch)
  (let ([n (char->integer ch)])
    (or (and (>= n 97) (<= n 122))
        (and (>= n 65) (<= n 90))
        (and (>= n 48) (<= n 57))
        (= n 95)
        (and (>= n 128) (not (char-whitespace? ch)))
        (hashset-contains? extra ch))))

;; ── Scanning ──────────────────────────────────────────────────────────────────

(define (bw/words-in-line word-char? line)
  (let loop ([chars (string->list line)] [run '()] [found '()])
    (cond
      [(null? chars) (if (null? run) found (cons (list->string (reverse run)) found))]
      [(word-char? (car chars)) (loop (cdr chars) (cons (car chars) run) found)]
      [(null? run) (loop (cdr chars) run found)]
      [else (loop (cdr chars) '() (cons (list->string (reverse run)) found))])))

(define (bw/union-words set words)
  (if (null? words)
      set
      (bw/union-words (hashset-insert set (car words)) (cdr words))))

;;; 0-indexed cursor line, or the top of the buffer for a background bid —
;;; see README.md's "Cursor-outward, line-windowed indexing".
(define (bw/anchor-line bid)
  (if (equal? bid (current-buffer))
      (let ([n (current-line-number)])
        (if n (- n 1) 0))
      0))

;;; One reindex tick — see README.md's "Cursor-outward, line-windowed
;;; indexing" for the walk shape and its persistent-hash/race-safety notes.
(define (bw/walk! bid word-char? fwd-line bwd-line)
  (let ([entry (bw/entry bid)])
    (when entry
      (let* ([total (buffer-line-count bid)]
             [fwd-hi (min total (+ fwd-line bw/lines-per-tick))]
             [fwd-lines (if (< fwd-line fwd-hi) (buffer-lines bid #:start fwd-line #:end fwd-hi) '())]
             [bwd-lo (max 0 (- bwd-line bw/lines-per-tick))]
             [bwd-lines (if (< bwd-lo bwd-line) (buffer-lines bid #:start bwd-lo #:end bwd-line) '())]
             [found (apply append
                            (map (lambda (l) (bw/words-in-line word-char? l))
                                 (append fwd-lines bwd-lines)))]
             [building (or (hash-ref entry "building") (hashset))])
        (bw/set! bid "building" (bw/union-words building found))
        (if (and (>= fwd-hi total) (<= bwd-lo 0))
            (begin
              (bw/set! bid "words" (or (hash-ref (bw/entry bid) "building") (hashset)))
              (bw/set! bid "building" #f)
              (bw/set! bid "timer" #f))
            (bw/set! bid "timer" (after 0 (lambda () (bw/walk! bid word-char? fwd-hi bwd-lo)))))))))

;;; Cancels any in-flight walk and restarts it fresh — see README.md's
;;; "Double-buffered cache".
(define (bw/reindex! bid)
  (let ([entry (bw/entry bid)])
    (when entry
      (bw/cancel-timer! entry)
      (let* ([anchor (bw/anchor-line bid)]
             [extra (list->hashset (string->list (get-option bid "word-chars")))])
        (bw/set! bid "building" (hashset))
        (bw/walk! bid (lambda (ch) (bw/word-char? extra ch)) anchor anchor)))))

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
    (completion-emit! id (bw/items bid prefix)))
  #:target 'buffer #:token 'word #:match bw/match)
