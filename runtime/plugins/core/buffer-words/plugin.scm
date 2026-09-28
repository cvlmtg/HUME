;;; core:buffer-words

(unless (member "core:stdlib" (declared-plugins))
  (error "core:buffer-words: requires core:stdlib — (declare-plugin! \"core:stdlib\") or (load-plugin! \"core:stdlib\") before (load-plugin! \"core:buffer-words\")"))

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
  (hash "words" #f "building" #f "timer" #f "gen" 0 "live-id" #f))

(define (bw/entry pane)
  (let ([table (unbox bw/*buffers*)]
        [key (buffer-key pane)])
    (and (hash-contains? table key) (hash-ref table key))))

(define (bw/install! pane entry)
  (set-box! bw/*buffers* (hash-insert (unbox bw/*buffers*) (buffer-key pane) entry)))

;;; Cancels `entry`'s pending tick, if any — see README.md's "Cursor-outward, line-windowed indexing".
(define (bw/cancel-timer! entry)
  (let ([timer (hash-ref entry "timer")])
    (when timer (cancel-timer! timer))))

(define (bw/forget! pane)
  (let ([entry (bw/entry pane)])
    (when entry (bw/cancel-timer! entry)))
  (set-box! bw/*buffers* (hash-remove (unbox bw/*buffers*) (buffer-key pane))))

;; ── Scanning ──────────────────────────────────────────────────────────────────

;;; The other-case form of `w`'s first letter, or `#f` when not "plain" enough
;;; to flip safely — see README.md's "Case twins".
(define (bw/case-twin w)
  (let ([rest (substring w 1 (string-length w))])
    (and (equal? rest (string-downcase rest))
         (let* ([head (string-ref w 0)]
                [flipped (if (equal? (char-upcase head) head) (char-downcase head) (char-upcase head))]
                [twin (string-append (string flipped) rest)])
           (and (not (equal? twin w)) twin)))))

;;; Inserts `w` into `set` plus its case twin (`bw/case-twin`) — see README.md's "Case twins".
(define (bw/add-word set w)
  (if (hashset-contains? set w)
      set
      (let ([twin (bw/case-twin w)])
        (if twin
            (hashset-insert (hashset-insert set w) twin)
            (hashset-insert set w)))))

;;; Folds every word `(split-words line wc)` finds across `lines` into `set`.
(define (bw/add-lines set lines wc)
  (foldl (lambda (line words) (foldl (lambda (w s) (bw/add-word s w)) words (split-words line wc)))
         set lines))

;;; 0-indexed cursor line, or the top of the buffer when no pane shows it — see README.md's "Cursor-outward, line-windowed indexing".
(define (bw/anchor-line pane)
  (let ([panes (buffer-panes pane)])
    (if (null? panes) 0 (buffer-cursor-line (car panes)))))

;;; Pure — see README.md's "Double-buffered cache"; `bw/walk!` installs and pushes.
(define (bw/finish-entry entry building)
  (hash-insert
    (hash-insert
      (hash-insert entry "words" (hashset->list building))
      "building" #f)
    "timer" #f))

(define (bw/continue-entry entry building timer)
  (hash-insert (hash-insert entry "building" building) "timer" timer))

;;; Pushes `entry`'s finished word list to the open menu, if any — see README.md's
;;; "Pushing a finished index to an open menu".
(define (bw/push-finished-answer! pane entry)
  (let ([id (hash-ref entry "live-id")])
    (when id
      (completion-emit! id (hash-ref entry "words"))
      (bw/install! pane (hash-insert entry "live-id" #f)))))

;;; One reindex tick — see README.md's "Cursor-outward, line-windowed indexing".
(define (bw/walk! pane gen wc fwd-line bwd-line)
  (let ([entry (bw/entry pane)])
    (when (and entry (= (hash-ref entry "gen") gen))
      (let* ([total (buffer-line-count pane)]
             [fwd-hi (min total (+ fwd-line bw/lines-per-tick))]
             [fwd-lines (if (< fwd-line fwd-hi) (buffer-lines pane #:start fwd-line #:end fwd-hi) '())]
             [bwd-hi (min bwd-line total)]
             [bwd-lo (max 0 (- bwd-hi bw/lines-per-tick))]
             [bwd-lines (if (< bwd-lo bwd-hi) (buffer-lines pane #:start bwd-lo #:end bwd-hi) '())]
             [building (bw/add-lines (bw/add-lines (or (hash-ref entry "building") (hashset)) fwd-lines wc)
                                      bwd-lines wc)])
        (if (and (>= fwd-hi total) (<= bwd-lo 0))
            (let ([finished (bw/finish-entry entry building)])
              (bw/install! pane finished)
              (bw/push-finished-answer! pane finished))
            (bw/install! pane
              (bw/continue-entry entry building
                                  (after! bw/tick-delay-ms
                                    (lambda () (bw/walk! pane gen wc fwd-hi bwd-lo))))))))))

;;; Cancels any in-flight walk and restarts it fresh — see README.md's
;;; "Double-buffered cache" and "Cursor-outward, line-windowed indexing".
(define (bw/reindex! pane)
  (unless (bw/entry pane)
    (when (buffer-live? pane)
      (bw/install! pane (bw/fresh-entry))))
  (let ([entry (bw/entry pane)])
    (when entry
      (bw/cancel-timer! entry)
      (let* ([anchor (bw/anchor-line pane)]
             [gen (+ (hash-ref entry "gen") 1)]
             [wc (get-buffer-option pane "word-chars")])
        (bw/install! pane (hash-insert (hash-insert entry "gen" gen) "building" #f))
        (bw/walk! pane gen wc anchor anchor)))))

;; ── Lifecycle ─────────────────────────────────────────────────────────────────

(register-hook! 'on-buffer-open
  (lambda (pane)
    (bw/forget! pane)
    (bw/reindex! pane)))

;;; See README.md's "Cursor-outward, line-windowed indexing" for the keying.
(define bw/schedule-reindex!
  (debounce-by 150 bw/reindex! #:key (lambda (p . _) (buffer-key p))))

(register-hook! 'on-text-changed
  (lambda (pane) (bw/schedule-reindex! pane)))

(register-hook! 'on-buffer-close
  (lambda (pane) (bw/forget! pane)))

;;; Reindexes every open buffer on a global `word-chars` change — see README.md's "`word-chars` invalidation".
(register-hook! 'on-option-change
  (lambda (key value)
    (when (equal? key "word-chars")
      (for-each bw/reindex! (buffers)))))

;; ── Completion source ─────────────────────────────────────────────────────────

;;; Reads the cache `bw/walk!` builds — see README.md's "Double-buffered cache" and "Matching".
(define (bw/items pane)
  (let ([entry (bw/entry pane)])
    (if (not entry)
        '()
        (let ([ready (hash-ref entry "words")])
          (if ready
              ready
              (let ([words (hash-ref entry "building")])
                (if words
                    (hashset->list words)
                    '())))))))

;;; Stashes `id` for `bw/push-finished-answer!` — see README.md's "Pushing a finished index to an open menu".
(define (bw/set-live-id! pane id)
  (let ([entry (bw/entry pane)])
    (when entry
      (bw/install! pane (hash-insert entry "live-id" id)))))

(register-completion-source! "buffer-words"
  (lambda (id pane prefix)
    (bw/set-live-id! pane id)
    (completion-emit! id (bw/items pane)))
  #:target 'buffer #:match bw/match)
