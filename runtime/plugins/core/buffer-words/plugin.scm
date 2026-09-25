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

;;; SSOT for a buffer's starting shape — see README.md's "Double-buffered
;;; cache". `"live-id"` is the completion-source's own state — see
;;; "Pushing a finished index to an open menu".
(define (bw/fresh-entry)
  (hash "words" #f "building" #f "timer" #f "gen" 0 "live-id" #f))

(define (bw/entry pane)
  (let ([table (unbox bw/*buffers*)]
        [key (buffer-key pane)])
    (and (hash-contains? table key) (hash-ref table key))))

(define (bw/install! pane entry)
  (set-box! bw/*buffers* (hash-insert (unbox bw/*buffers*) (buffer-key pane) entry)))

;;; Cancels `entry`'s pending tick, if any — see README.md's "Cursor-
;;; outward, line-windowed indexing" for why `"gen"` is what actually makes
;;; staleness safe.
(define (bw/cancel-timer! entry)
  (let ([timer (hash-ref entry "timer")])
    (when timer (cancel-timer! timer))))

(define (bw/forget! pane)
  (let ([entry (bw/entry pane)])
    (when entry (bw/cancel-timer! entry)))
  (set-box! bw/*buffers* (hash-remove (unbox bw/*buffers*) (buffer-key pane))))

;; ── Scanning ──────────────────────────────────────────────────────────────────

;;; The other-case form of `w`'s first letter, or `#f` when `w` isn't
;;; "plain" enough for the flip to be safe — see README.md's "Case twins".
;;; Plain means everything after the first letter is already lowercase: an
;;; all-lowercase word (`apply`) or a Titlecase one (`Apply`). A word with an
;;; inner capital (`HashMap`, `iPhone`) or in ALL-CAPS (`MAX_LEN`) returns
;;; `#f` — flipping only the head would destroy case information the tail
;;; still carries, not merely restate it.
;;;
;;; Steel has no `char-upper-case?` to pick a flip direction directly, so the
;;; head's own case is read off which of `char-upcase`/`char-downcase`
;;; changes it: an already-uppercase head is a `char-upcase` no-op, so the
;;; flip goes the other way. A caseless head (`123abc`, `_foo`) is a no-op
;;; both ways, so `flipped` comes back equal to `head` and the final
;;; `equal?` check below drops it — same outcome as the empty-string case
;;; (`string-ref`/`substring` on `""` would error, never reached because
;;; `rest` of a 1-char `w` is `""`, and `equal? "" (string-downcase "")` is
;;; true, so the case only ever reaches the no-op check, never a bounds error).
(define (bw/case-twin w)
  (let ([rest (substring w 1 (string-length w))])
    (and (equal? rest (string-downcase rest))
         (let* ([head (string-ref w 0)]
                [flipped (if (equal? (char-upcase head) head) (char-downcase head) (char-upcase head))]
                [twin (string-append (string flipped) rest)])
           (and (not (equal? twin w)) twin)))))

;;; Inserts `w` into `set`, plus its case twin (`bw/case-twin`) when `w` is
;;; plain — skipped when `w` is already present, so a repeated word pays for
;;; its twin once per walk rather than once per occurrence.
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

;;; 0-indexed cursor line, or the top of the buffer when no pane shows it.
;;; See README.md's "Cursor-outward, line-windowed indexing".
(define (bw/anchor-line pane)
  (let ([panes (buffer-panes pane)])
    (if (null? panes) 0 (- (buffer-cursor-line (car panes)) 1))))

;;; See README.md's "Double-buffered cache". Pure — does not install `pane`'s
;;; entry or push anything; `bw/walk!` (the one caller) does both once it
;;; has the finished entry in hand.
(define (bw/finish-entry entry building)
  (hash-insert
    (hash-insert
      (hash-insert entry "words" (hashset->list building))
      "building" #f)
    "timer" #f))

(define (bw/continue-entry entry building timer)
  (hash-insert (hash-insert entry "building" building) "timer" timer))

;;; Pushes `entry`'s freshly finished word list to whatever completion
;;; invocation is still open on it, if any — the id `bw/set-live-id!`
;;; stashed when the completion source itself last ran. See README.md's
;;; "Pushing a finished index to an open menu". A stale id — no menu open,
;;; or a newer trigger already replaced it — is silently dropped by
;;; `completion-emit!` itself; nothing here needs to know which. Clears
;;; `"live-id"` once used: the completion source only answers (and so only
;;; sets a fresh id) once per session, not once per keystroke, so without
;;; this a *second* background walk finishing while the same menu is still
;;; open would re-emit under the same still-live id and reset the menu's
;;; selection a second time, with no new trigger to justify it.
(define (bw/push-finished-answer! pane entry)
  (let ([id (hash-ref entry "live-id")])
    (when id
      (completion-emit! id (hash-ref entry "words"))
      (bw/install! pane (hash-insert entry "live-id" #f)))))

;;; One reindex tick — see README.md's "Cursor-outward, line-windowed
;;; indexing".
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
                                  (after bw/tick-delay-ms
                                    (lambda () (bw/walk! pane gen wc fwd-hi bwd-lo))))))))))

;;; Cancels any in-flight walk and restarts it fresh, resurrecting `pane`'s
;;; entry first if missing — see README.md's "Double-buffered cache" and
;;; "Cursor-outward, line-windowed indexing". Clears `"building"` along with
;;; bumping `"gen"`: `bw/walk!` reads it back as its own starting set, and
;;; without this a cancelled walk's partial (and possibly now-stale — the
;;; reindex was likely triggered by an edit) set would carry into the fresh
;;; one instead of that one starting empty.
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

;;; Reindexes every open buffer on a global `word-chars` change — see
;;; README.md's "word-chars invalidation" for the buffer-scoped gap this
;;; doesn't close.
(register-hook! 'on-option-change
  (lambda (key value)
    (when (equal? key "word-chars")
      (for-each bw/reindex! (buffers)))))

;; ── Completion source ─────────────────────────────────────────────────────────

;;; Reads the cache `bw/walk!` builds — see README.md's "Double-buffered
;;; cache" and "Matching". The typed word itself is not filtered out
;;; here — the editor's own ranking drops any item that's a no-op against
;;; what's typed, the same rule every other completion source relies on.
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

;;; Stashes `id` as the invocation a still-in-progress walk should push its
;;; finished answer to — see `bw/push-finished-answer!` and README.md's
;;; "Pushing a finished index to an open menu". A no-op if `pane` has no
;;; entry (the source answered empty and there's nothing to track).
(define (bw/set-live-id! pane id)
  (let ([entry (bw/entry pane)])
    (when entry
      (bw/install! pane (hash-insert entry "live-id" id)))))

(register-completion-source! "buffer-words"
  (lambda (id pane prefix)
    (bw/set-live-id! pane id)
    (completion-emit! id (bw/items pane)))
  #:target 'buffer #:match bw/match)
