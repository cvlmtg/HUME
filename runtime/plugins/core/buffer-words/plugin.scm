;;; core:buffer-words

(unless (member "core:stdlib" (declared-plugins))
  (error "core:buffer-words: requires core:stdlib — (declare-plugin \"core:stdlib\") or (load-plugin \"core:stdlib\") before (load-plugin \"core:buffer-words\")"))

;; ── Config ────────────────────────────────────────────────────────────────────

(define bw/cfg (plugin-config))
(define bw/match (call! "stdlib/config-enum" "core:buffer-words" bw/cfg "match" 'string '(string fuzzy)))
(define bw/lines-per-tick (call! "stdlib/config-integer" "core:buffer-words" bw/cfg "lines" 200 1))

;; Delay between walk ticks. Not 0 — see README.md's "Cursor-outward,
;; line-windowed indexing": a zero-delay `(after 0 …)` is already due the
;; instant it's scheduled, which pins the event loop's wake timeout at zero
;; and repaints a full unchanged frame once per tick for the whole walk.
(define bw/tick-delay-ms 16)

;; ── State ─────────────────────────────────────────────────────────────────────
;; bid -> (hash "words" <hashset-or-#f> "building" <hashset-or-#f> "timer" <id-or-#f>
;;              "gen" <integer>)
;; See README.md's "Double-buffered cache".

(define bw/*buffers* (box (hash)))

;;; SSOT for a buffer's starting shape — see `bw/init-buffer!` and
;;; `bw/reindex!`'s resurrection branch.
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

;;; Cancels `entry`'s in-flight walk, if any — the one pending tick, since
;;; each tick is what schedules the next. Saves a wasted tick when it lands
;;; in time; `"gen"` (bumped by `bw/reindex!`, checked by `bw/walk!`) is what
;;; makes staleness safe even when it doesn't — see README.md's "Cursor-
;;; outward, line-windowed indexing".
(define (bw/cancel-timer! entry)
  (let ([timer (hash-ref entry "timer")])
    (when timer (cancel-timer! timer))))

;;; Cancels any in-flight walk before installing a fresh entry — see
;;; README.md's "Cursor-outward, line-windowed indexing".
(define (bw/init-buffer! bid)
  (bw/forget! bid)
  (set-box! bw/*buffers* (hash-insert (unbox bw/*buffers*) bid (bw/fresh-entry))))

(define (bw/forget! bid)
  (let ([entry (bw/entry bid)])
    (when entry (bw/cancel-timer! entry)))
  (set-box! bw/*buffers* (hash-remove (unbox bw/*buffers*) bid)))

;; ── Scanning ──────────────────────────────────────────────────────────────────
;;
;; Classification itself is `split-words` — a native builtin, not Steel: see
;; README.md's "Non-ASCII words". It's the real `WordChars`/`CharClass`
;; machinery `w`/`b` motions and text objects already use, so a word found
;; here is exactly what one of those would select.

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
;;; `gen` is the generation `bw/reindex!` started this walk under — checked
;;; against the entry's own `"gen"` first, since a tick already queued past
;;; `bw/cancel-timer!`'s reach must not touch an entry a newer walk now owns
;;; (see the "gen" comment on `bw/cancel-timer!`).
(define (bw/walk! bid word-chars gen fwd-line bwd-line)
  (let ([entry (bw/entry bid)])
    (when (and entry (= (hash-ref entry "gen") gen))
      (let* ([total (buffer-line-count bid)]
             [fwd-hi (min total (+ fwd-line bw/lines-per-tick))]
             [fwd-lines (if (< fwd-line fwd-hi) (buffer-lines bid #:start fwd-line #:end fwd-hi) '())]
             [bwd-hi (min bwd-line total)]
             [bwd-lo (max 0 (- bwd-hi bw/lines-per-tick))]
             [bwd-lines (if (< bwd-lo bwd-hi) (buffer-lines bid #:start bwd-lo #:end bwd-hi) '())]
             ;; `(if (null? fwd-lines) bwd-lines (append fwd-lines bwd-lines))`,
             ;; not a direct `(append fwd-lines bwd-lines)`: steel-core 0.8.2
             ;; silently drops every `bwd-lines` entry past the 4th when the
             ;; *first* argument to a 2-argument `append` is the literal empty
             ;; list — the same bug guarded at
             ;; runtime/plugins/core/git-diff/render.scm:99-113, which has the
             ;; full explanation. `fwd-lines` is `'()` on every tick once the
             ;; forward side reaches `total`, which is most ticks on a buffer
             ;; anchored near its end.
             [scanned (if (null? fwd-lines) bwd-lines (append fwd-lines bwd-lines))]
             [found (apply append (map (lambda (l) (split-words l word-chars)) scanned))]
             [building (or (hash-ref entry "building") (hashset))])
        (bw/set! bid "building" (bw/union-words building found))
        (if (and (>= fwd-hi total) (<= bwd-lo 0))
            (begin
              (bw/set! bid "words" (or (hash-ref (bw/entry bid) "building") (hashset)))
              (bw/set! bid "building" #f)
              (bw/set! bid "timer" #f))
            (bw/set! bid "timer"
                     (after bw/tick-delay-ms
                       (lambda () (bw/walk! bid word-chars gen fwd-hi bwd-lo)))))))))

;;; Cancels any in-flight walk and restarts it fresh — see README.md's
;;; "Double-buffered cache". Resurrects `bid`'s entry first if it's missing
;;; (a buffer-close reusing the `BufferId` for a fresh scratch fires no
;;; `on-buffer-open`, only the `on-text-changed` this plugin already reacts
;;; to — see README.md's "Cursor-outward, line-windowed indexing"), gated on
;;; `bid` still being a live buffer so a debounced reindex that outlives a
;;; genuine close can't resurrect dead state, the same reason
;;; `git-diff/entry-set!` no-ops instead of resurrecting
;;; (runtime/plugins/core/git-diff/state.scm).
;;;
;;; The in-progress `"building"` set survives a restart rather than being
;;; reset to empty: the walk is monotone (a tick only ever adds words), so an
;;; edit that keeps interrupting a large walk near the cursor still makes
;;; forward progress on the far ends instead of losing them every time.
(define (bw/reindex! bid)
  (unless (bw/entry bid)
    (when (member bid (buffers))
      (set-box! bw/*buffers* (hash-insert (unbox bw/*buffers*) bid (bw/fresh-entry)))))
  (let ([entry (bw/entry bid)])
    (when entry
      (bw/cancel-timer! entry)
      (let* ([anchor (bw/anchor-line bid)]
             [word-chars (get-option bid "word-chars")]
             [gen (+ (hash-ref entry "gen") 1)])
        (bw/set! bid "gen" gen)
        (bw/walk! bid word-chars gen anchor anchor)))))

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

;;; `word-chars` is read once per reindex and baked into the cached set (see
;;; `bw/reindex!`), so a global change needs an explicit rebuild — otherwise
;;; every open buffer's index would silently keep classifying by the old
;;; value until its next edit. See README.md's "Cursor-outward, line-windowed
;;; indexing" for the buffer-scoped `:set buffer word-chars=…` gap this
;;; doesn't close — `on-option-change` never fires for a buffer-scoped write.
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
