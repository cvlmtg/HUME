;;; core:buffer-words

;; ── Config ────────────────────────────────────────────────────────────────────

(define bw/cfg (plugin-config))
(define bw/match (call! "stdlib/config-enum" "core:buffer-words" bw/cfg 'match 'string '(string fuzzy)))
(define bw/chars-per-tick (call! "stdlib/config-integer" "core:buffer-words" bw/cfg 'chars 8000 1))

;;; Not 0 — see README.md's "Cursor-outward, line-windowed indexing".
(define bw/tick-delay-ms 16)

;;; See README.md's "Updating after an edit".
(define bw/max-word-checks 32)

;; ── State ─────────────────────────────────────────────────────────────────────

(define bw/*buffers* (box (hash)))

;;; SSOT for a buffer's starting shape — see README.md's "Double-buffered cache".
(define (bw/fresh-entry)
  (hash "words" #f "set" #f "seen" #f "building" #f "timer" #f "gen" 0 "live-id" #f))

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

;;; Folds every word in `lines` into `set` — see README.md's "Non-ASCII words".
(define (bw/add-lines set pane lines)
  (foldl (lambda (line words)
           (foldl (lambda (w s) (bw/add-word s w)) words (call! "stdlib/split-words" pane line)))
         set lines))

;;; 0-indexed cursor line, or the top of the buffer when no pane shows it — see README.md's "Cursor-outward, line-windowed indexing".
(define (bw/anchor-line pane)
  (let ([panes (buffer-panes pane)])
    (if (null? panes) 0 (buffer-cursor-line (car panes)))))

;; ── Updating after an edit ────────────────────────────────────────────────────

;;; Every line of `changes`' hunks under `key` ('old-lines or 'new-lines).
(define (bw/hunk-lines changes key)
  (call! "stdlib/concat" (map (lambda (h) (hash-ref h key)) changes)))

(define (bw/line-chars lines)
  (foldl (lambda (line n) (+ n (string-length line))) 0 lines))

;;; The distinct words of `lines`, as a hashset.
(define (bw/words-in pane lines)
  (apply hashset
         (call! "stdlib/concat" (map (lambda (line) (call! "stdlib/split-words" pane line)) lines))))

;;; `words` plus each one's case twin — see README.md's "Case twins".
(define (bw/with-twins words)
  (foldl (lambda (w acc)
           (let ([twin (bw/case-twin w)])
             (if twin (cons w (cons twin acc)) (cons w acc))))
         '() words))

;;; Whether `w` or its case twin still occurs as a word — see README.md's "Updating after an edit".
(define (bw/present? pane w)
  (let* ([head (string-ref w 0)]
         [twin (bw/case-twin w)]
         [form (if (and twin (not (equal? head (char-downcase head)))) twin w)])
    (and (buffer-search pane (word-search-pattern pane form)) #t)))

;;; `set` updated for `changes`: `set` itself when no word joins or leaves,
;;; `#f` when a restarted walk costs less — see README.md's "Updating after an edit".
(define (bw/updated-set set pane changes)
  (let ([old-lines (bw/hunk-lines changes 'old-lines)]
        [new-lines (bw/hunk-lines changes 'new-lines)])
    (and (<= (+ (bw/line-chars old-lines) (bw/line-chars new-lines)) bw/chars-per-tick)
         (let* ([new-words (bw/words-in pane new-lines)]
                [candidates (filter (lambda (w) (and (hashset-contains? set w)
                                                     (not (hashset-contains? new-words w))))
                                    (hashset->list (bw/words-in pane old-lines)))])
           (and (<= (length candidates) bw/max-word-checks)
                (let ([gone (filter (lambda (w) (not (bw/present? pane w))) candidates)]
                      [added (filter (lambda (w) (not (hashset-contains? set w)))
                                     (bw/with-twins (hashset->list new-words)))])
                  (if (and (null? gone) (null? added))
                      set
                      ;; See README.md's "Updating after an edit" for the intersection.
                      (hashset-union
                        (hashset-difference
                          set (hashset-intersection set (apply hashset (bw/with-twins gone))))
                        (apply hashset added)))))))))

;;; Walk position `p`, a line of the text before `changes`, in the text after
;;; them. `side` is 'forward or 'backward, the end of a hunk spanning `p` it
;;; lands on — see README.md's "Updating after an edit".
(define (bw/shift-line changes p side)
  (let ([above (foldl (lambda (h acc) (if (< (hash-ref h 'old-start-line) p) h acc)) #f changes)])
    (cond [(not above) p]
          [(<= (hash-ref above 'old-end-line) p)
           (+ p (- (hash-ref above 'new-end-line) (hash-ref above 'old-end-line)))]
          [(equal? side 'forward) (hash-ref above 'new-end-line)]
          [else (hash-ref above 'new-start-line)])))

;; ── Walking ───────────────────────────────────────────────────────────────────

;;; Pure — see README.md's "Double-buffered cache"; `bw/walk!` installs and pushes.
(define (bw/finish-entry entry building)
  (hash-insert
    (hash-insert
      (hash-insert
        (hash-insert entry "words" (hashset->list building))
        "set" building)
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

;;; End line (exclusive) of the forward window from `line` (< `total`) — see README.md's "Character budget".
(define (bw/forward-end pane line total len)
  (let ([target (+ (line->char-offset pane line) bw/chars-per-tick)])
    (if (>= target len)
        total
        (max (+ line 1) (char-offset->line pane target)))))

;;; Start line of the backward window ending before `hi` (> 0) — see README.md's "Character budget".
(define (bw/backward-start pane hi total len)
  (let ([target (- (if (< hi total) (line->char-offset pane hi) len) bw/chars-per-tick)])
    (if (<= target 0)
        0
        (min (- hi 1) (+ (char-offset->line pane (- target 1)) 1)))))

;;; One walk tick, caught up with any edit since the last one — see README.md's
;;; "Cursor-outward, line-windowed indexing" and "Updating after an edit".
(define (bw/walk! pane gen fwd-from bwd-from)
  (let ([entry (bw/entry pane)])
    (when (and entry (= (hash-ref entry "gen") gen))
      (let* ([now (buffer-generation pane)]
             [changes (if (equal? now (hash-ref entry "seen"))
                          '()
                          (buffer-changes-since pane (hash-ref entry "seen")))]
             [caught-up (and changes
                             (bw/updated-set (or (hash-ref entry "building") (hashset)) pane changes))])
        (if (not caught-up)
            (bw/reindex! pane)
            (let* ([fwd-line (bw/shift-line changes fwd-from 'forward)]
                   [bwd-line (bw/shift-line changes bwd-from 'backward)]
                   [total (buffer-line-count pane)]
                   [len (buffer-char-count pane)]
                   [fwd-hi (if (< fwd-line total) (bw/forward-end pane fwd-line total len) total)]
                   [fwd-lines (if (< fwd-line fwd-hi) (buffer-lines pane #:range (hash 'start-line fwd-line 'end-line fwd-hi)) '())]
                   [bwd-hi (min bwd-line total)]
                   [bwd-lo (if (> bwd-hi 0) (bw/backward-start pane bwd-hi total len) 0)]
                   [bwd-lines (if (< bwd-lo bwd-hi) (buffer-lines pane #:range (hash 'start-line bwd-lo 'end-line bwd-hi)) '())]
                   [building (bw/add-lines (bw/add-lines caught-up pane fwd-lines) pane bwd-lines)]
                   [current (hash-insert entry "seen" now)])
              (if (and (>= fwd-hi total) (<= bwd-lo 0))
                  (let ([finished (bw/finish-entry current building)])
                    (bw/install! pane finished)
                    (bw/push-finished-answer! pane finished))
                  (bw/install! pane
                    (bw/continue-entry current building
                                        (after! bw/tick-delay-ms
                                          (lambda () (bw/walk! pane gen fwd-hi bwd-lo))))))))))))

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
             [gen (+ (hash-ref entry "gen") 1)])
        (bw/install! pane
          (hash-insert (hash-insert (hash-insert entry "gen" gen) "building" #f)
                       "seen" (buffer-generation pane)))
        (bw/walk! pane gen anchor anchor)))))

;;; Brings `pane`'s complete index up to date after edits; a running walk
;;; catches up on its own — see README.md's "Updating after an edit".
(define (bw/refresh! pane)
  (let ([entry (bw/entry pane)])
    (unless (and entry (hash-ref entry "building"))
      (let* ([seen (and entry (hash-ref entry "seen"))]
             [changes (and seen (buffer-changes-since pane seen))]
             [set (and changes (hash-ref entry "set"))]
             [updated (and changes (bw/updated-set set pane changes))]
             [current (and updated (hash-insert entry "seen" (buffer-generation pane)))])
        (cond [(not updated) (bw/reindex! pane)]
              [(eq? updated set) (bw/install! pane current)]
              [else (bw/install! pane
                      (hash-insert (hash-insert current "set" updated) "words" (hashset->list updated)))])))))

;; ── Lifecycle ─────────────────────────────────────────────────────────────────

(register-hook! 'on-buffer-open
  (lambda (pane)
    (bw/forget! pane)
    (bw/reindex! pane)))

;;; See README.md's "Cursor-outward, line-windowed indexing" for the keying.
(define bw/schedule-refresh!
  (debounce-by 150 bw/refresh! #:key (lambda (p . _) (buffer-key p))))

(register-hook! 'on-text-changed
  (lambda (pane changes) (bw/schedule-refresh! pane)))

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

;;; Whether a walk over `pane`'s buffer is in progress — see README.md's "Pushing a finished index to an open menu".
(define (bw/building? pane)
  (let ([entry (bw/entry pane)])
    (and entry (hash-ref entry "building") #t)))

;;; Stashes `id` for `bw/push-finished-answer!` — see README.md's "Pushing a finished index to an open menu".
(define (bw/set-live-id! pane id)
  (let ([entry (bw/entry pane)])
    (when entry
      (bw/install! pane (hash-insert entry "live-id" id)))))

(register-completion-source! "buffer-words"
  (lambda (id pane prefix)
    (let ([building? (bw/building? pane)])
      (bw/set-live-id! pane (and building? id))
      (completion-emit! id (bw/items pane) #:more building?)))
  #:target 'buffer #:match bw/match)
