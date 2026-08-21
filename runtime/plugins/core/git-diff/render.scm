;;; core:git-diff — render.scm
;;;
;;; Pure `hunks → decoration records` functions, one per rendering, each
;;; ending in exactly one setter call (`render-inline!` is the one
;;; exception — see its own comment). State stores the verbatim hunk tuples
;;; `diff-buffer-lines` returns, never a derived shape (state.scm's
;;; additivity invariant), so every function here is a pure view over that
;;; one shared shape: gutter signs, virtual deleted lines + word highlights,
;;; and the full-row background tint.

(provide git-diff/render-signs! git-diff/render-inline! git-diff/render-line-bgs!
         git-diff/render-for!)

;;; Shared source namespace for every setter this plugin calls — a plain
;;; feature-scoped name, not the `core:git-diff` plugin id, matching
;;; `core:lsp`'s own decoration sources (`"lsp-diagnostics"`,
;;; `"lsp-inlay-hints"`).
(define git-diff/*source* "git-diff")

;; ── Signs ────────────────────────────────────────────────────────────────────

;;; All plugin signs render at the same priority — there is only one
;;; producer (this plugin) contending for this buffer's sign slot against
;;; LSP diagnostics, which live in a separate map entirely (HUME's
;;; decoration_providers.rs), so relative ordering among *our own* signs
;;; never matters.
(define git-diff/*sign-priority* 0)

;;; One `(text scope)`-tagged sign per line in `[new-start, new-start +
;;; new-count)` — the shared body of the pure-add and change cases below,
;;; which differ only in which text/scope they mark the new-side range with.
(define (git-diff/line-signs new-start new-count text scope)
  (map (lambda (line) (list line text scope git-diff/*sign-priority*))
       (range new-start (+ new-start new-count))))

;;; One `diff-buffer-lines` hunk `(old-start old-count new-start new-count
;;; old-lines new-lines)` -> a list of `(line text scope priority)` sign
;;; entries, one per changed line (VSCode/gitsigns density, not one per
;;; hunk) — a 20-line paste should show 20 `+` marks, not one.
(define (git-diff/hunk->signs hunk)
  (let* ([old-count (list-ref hunk 1)]
         [new-start (list-ref hunk 2)]
         [new-count (list-ref hunk 3)])
    (cond
      ;; Pure deletion: no new-side lines to anchor on, so mark the line
      ;; above the gap instead — gitsigns' convention. `(- new-start 1)`
      ;; rather than `new-start` itself also sidesteps an out-of-range
      ;; `set-signs!` call for a deletion at end of file, where `new-start`
      ;; equals the buffer's line count; `(max 0 …)` covers a deletion at
      ;; line 0, where there is no line above.
      [(= new-count 0)
       (list (list (max 0 (- new-start 1)) "-" "diff.minus.gutter" git-diff/*sign-priority*))]
      [(= old-count 0) (git-diff/line-signs new-start new-count "+" "diff.plus.gutter")]
      [else (git-diff/line-signs new-start new-count "~" "diff.delta.gutter")])))

;;; `hunks → (set-signs! …)`, the one setter call this rendering makes.
;;; `(apply append …)`, not `flatten` — a sign entry is itself a list, and
;;; `flatten` would tear each one apart instead of just concatenating the
;;; per-hunk lists. An empty `hunks` list clears the gutter for `bid`
;;; (`set-signs!` replaces `source`'s signs wholesale), so this doubles as
;;; the plugin's clear function — no separate one is needed.
(define (git-diff/render-signs! bid hunks)
  (set-signs! git-diff/*source* bid (apply append (map git-diff/hunk->signs hunks))))

;; ── Inline: deleted lines + word highlights ─────────────────────────────────

;;; True for a char `set-virtual-lines!` rejects in `'text`
;;; (`c.is_control()` in `hume-scripting/src/builtins/decorations.rs`, Rust's
;;; `char::is_control`, i.e. Unicode category `Cc`: U+0000–U+001F and
;;; U+007F–U+009F). Steel has no `char-control?` builtin, so both ranges are
;;; checked by hand rather than delegating.
(define (git-diff/control-char? c)
  (let ([n (char->integer c)])
    (or (< n 32) (and (>= n 127) (< n 160)))))

;;; Fast path for `git-diff/expand-tabs`/`git-diff/expanded-offset`: most
;;; lines have no control character (only tab-indented ones do), so skip
;;; the walk and use the line as-is.
(define (git-diff/needs-expansion? text)
  (let loop ([i 0])
    (cond [(= i (string-length text)) #f]
          [(git-diff/control-char? (string-ref text i)) #t]
          [else (loop (+ i 1))])))

;;; Next tab stop at or after column `col`, `tab-width` columns apart —
;;; shared by `expand-tabs` (building the expanded string) and
;;; `expanded-offset` (mapping one raw offset into it), so the two always
;;; agree on where a given tab lands.
(define (git-diff/tab-stop col tab-width)
  (* tab-width (+ 1 (quotient col tab-width))))

;;; `text` with every control character replaced by something safe for
;;; `set-virtual-lines!`'s `'text`, which raises on `\t` and friends: a tab
;;; expands to spaces up to the next `tab-width` column stop (matching the
;;; live buffer's own rendering, not a fixed width); anything else becomes
;;; one literal space. Only called once `git-diff/needs-expansion?` is
;;; already known `#t`.
(define (git-diff/expand-tabs text tab-width)
  (let loop ([i 0] [col 0] [acc '()])
    (if (= i (string-length text))
        (list->string (reverse acc))
        (let ([c (string-ref text i)])
          (if (char=? c #\tab)
              (let ([stop (git-diff/tab-stop col tab-width)])
                (loop (+ i 1) stop (append (map (lambda (_) #\space) (range 0 (- stop col))) acc)))
              (loop (+ i 1) (+ col 1) (cons (if (git-diff/control-char? c) #\space c) acc)))))))

;;; The column raw char offset `idx` into `text` lands at after the same
;;; expansion `expand-tabs` performs — walked independently rather than
;;; sharing state with it, since segment bounds are only needed for a line
;;; that actually has segments, not for every expanded line.
(define (git-diff/expanded-offset text tab-width idx)
  (let loop ([i 0] [col 0])
    (if (= i idx)
        col
        (loop (+ i 1)
              (if (char=? (string-ref text i) #\tab)
                  (git-diff/tab-stop col tab-width)
                  (+ col 1))))))

;;; Where a hunk's removed old-side lines attach, as a `(kind . line)` pair.
;;; `'after (- new-start 1)` when a preceding line exists — it renders at
;;; the same visual position `'before new-start` would (adjacent lines'
;;; `'after`/`'before` slots are the same gap), but stays valid when
;;; `new-start` equals the buffer's content line count: a deletion at end
;;; of file, where `'before new-start` would address the phantom trailing
;;; line and raise. `'before 0` only when there is no preceding line — a
;;; deletion at the very start of the buffer.
(define (git-diff/hunk-anchor new-start)
  (if (= new-start 0)
      (cons 'before 0)
      (cons 'after (- new-start 1))))

;;; Base hashmap shared by every removed-line virtual row: `'line`/`'anchor`
;;; from `anchor`, `'scope "diff.minus"` for the whole row (covers every
;;; char `segments` doesn't cover), `'segments` added only when non-empty —
;;; an empty list is a valid `set-virtual-lines!` value, but omitting the
;;; key when there's nothing to highlight keeps the hash to what's used.
(define (git-diff/virtual-line-hash text anchor segments)
  ;; Symbol keys, not strings — `set-virtual-lines!` looks each field up as
  ;; `(SteelVal::SymbolV k)`; a string key is silently a different, unknown
  ;; key and raises "hashmap key must be a symbol".
  (let ([base (hash 'line (cdr anchor) 'text text 'anchor (car anchor) 'scope "diff.minus")])
    (if (null? segments) base (hash-insert base 'segments segments))))

;;; A whole removed line with no word-level detail — a pure deletion, an
;;; unpaired excess old line inside a change hunk, or a pair whose
;;; `diff-words` call hit its deadline (coarse result, not trustworthy
;;; enough to show per-word).
(define (git-diff/plain-virtual-line old-line anchor tab-width)
  (let ([text (if (git-diff/needs-expansion? old-line)
                   (git-diff/expand-tabs old-line tab-width)
                   old-line)])
    (git-diff/virtual-line-hash text anchor '())))

;;; `old-line`'s virtual row with word-del `'segments` built from
;;; `diff-words`' hunks (`(old-start old-end new-start new-end old-text
;;; new-text)` tuples — only the first two fields are this line's own char
;;; offsets). Filtered to hunks that actually remove something from the OLD
;;; side (`old-start < old-end`): a hunk that's a pure insertion on this
;;; line has nothing to underline here, and a zero-width segment would
;;; raise (`set-virtual-lines!`'s `start < end` check). Segment bounds are
;;; remapped through the same tab expansion as `'text` itself — the host
;;; validates `'segments` against the *expanded* text's length, not the raw
;;; line's.
(define (git-diff/virtual-line-with-segments old-line anchor tab-width word-hunks)
  (let* ([expand? (git-diff/needs-expansion? old-line)]
         [text (if expand? (git-diff/expand-tabs old-line tab-width) old-line)]
         [removals (filter (lambda (wh) (< (list-ref wh 0) (list-ref wh 1))) word-hunks)]
         [segments (map (lambda (wh)
                          (let ([s (list-ref wh 0)] [e (list-ref wh 1)])
                            (list (if expand? (git-diff/expanded-offset old-line tab-width s) s)
                                  (if expand? (git-diff/expanded-offset old-line tab-width e) e)
                                  "diff.minus.word")))
                        removals)])
    (git-diff/virtual-line-hash text anchor segments)))

;;; `word-hunks`' new-side fields (char offsets into the live line,
;;; unaffected by tab expansion — the live buffer still has real tabs) ->
;;; `(start end scope)` triples in *buffer* char offsets, since
;;; `set-extra-highlights!` addresses the whole buffer, not one line.
;;; Filtered the same way as the old side and for the same reason: a hunk
;;; that's a pure deletion on this line (`new-start == new-end`) adds
;;; nothing to underline on the live side, and a zero-width span would
;;; raise.
(define (git-diff/word-hunks->new-side-spans line-offset word-hunks)
  (let ([additions (filter (lambda (wh) (< (list-ref wh 2) (list-ref wh 3))) word-hunks)])
    (map (lambda (wh)
           (list (+ line-offset (list-ref wh 2))
                 (+ line-offset (list-ref wh 3))
                 "diff.plus.word"))
         additions)))

;;; Char offset where each of the first `paired-count` `new-lines` starts,
;;; without one `line->offset` host call per line — only the hunk's first
;;; new-side line needs it (`new-start`); every later line in this
;;; LF-normalized buffer is exactly its predecessor's length plus one `\n`
;;; further along.
(define (git-diff/paired-line-offsets bid new-start new-lines paired-count)
  (let ([base (line->offset bid new-start)])
    (let loop ([i 0] [offset base] [lines new-lines] [acc '()])
      (if (= i paired-count)
          (reverse acc)
          (loop (+ i 1) (+ offset (string-length (car lines)) 1) (cdr lines) (cons offset acc))))))

;;; One paired (old-line . new-line) -> `(virtual-line . spans)`, one
;;; `diff-words` call shared by both — see `render-inline!`'s comment on
;;; why this makes two decoration kinds instead of one.
(define (git-diff/paired-line->vl+spans old-line new-line line-offset anchor tab-width)
  (let* ([result (diff-words old-line new-line)]
         [word-hunks (car result)]
         [deadline-hit? (cdr result)])
    (if deadline-hit?
        (cons (git-diff/plain-virtual-line old-line anchor tab-width) '())
        (cons (git-diff/virtual-line-with-segments old-line anchor tab-width word-hunks)
              (git-diff/word-hunks->new-side-spans line-offset word-hunks)))))

;;; One hunk's removed old-side lines -> `(virtual-lines . spans)`.
;;; `old-lines[0, paired-count)` have a same-index `new-lines` counterpart
;;; to word-diff against; any remainder (a hunk removing more lines than it
;;; adds) gets a plain whole-line row, same treatment as a pure deletion.
(define (git-diff/hunk-old-lines->virtual+spans bid old-lines new-lines new-start paired-count anchor tab-width)
  (let* ([offsets (if (> paired-count 0)
                       (git-diff/paired-line-offsets bid new-start new-lines paired-count)
                       '())]
         ;; Walks `old-lines`/`new-lines`/`offsets` together via `cdr`, not
         ;; `list-ref` by index — Steel lists are linked, so indexing each of
         ;; three lists per iteration made this quadratic in `paired-count`.
         [paired (let loop ([olds old-lines] [news new-lines] [offs offsets] [n paired-count] [acc '()])
                   (if (= n 0)
                       (reverse acc)
                       (loop (cdr olds) (cdr news) (cdr offs) (- n 1)
                             (cons (git-diff/paired-line->vl+spans
                                     (car olds) (car news) (car offs) anchor tab-width)
                                   acc))))]
         [unpaired (map (lambda (old-line)
                          (cons (git-diff/plain-virtual-line old-line anchor tab-width) '()))
                        (list-tail old-lines paired-count))]
         [all (append paired unpaired)])
    (cons (map car all) (apply append (map cdr all)))))

;;; One hunk -> `(virtual-lines . spans)` for `render-inline!`. A pure
;;; addition (`old-count` 0) contributes nothing here — nothing was removed
;;; to show as a virtual row, and `render-line-bgs!` alone covers its
;;; new-side tint. `tab-width` is a `render-inline!`-wide constant, not
;;; per-hunk state — the caller reads it once and passes it down, rather
;;; than this making a `get-option` host call per hunk.
(define (git-diff/hunk-inline-data bid hunk tab-width)
  (let* ([old-count (list-ref hunk 1)]
         [new-start (list-ref hunk 2)]
         [new-count (list-ref hunk 3)]
         [old-lines (list-ref hunk 4)]
         [new-lines (list-ref hunk 5)])
    (if (= old-count 0)
        (cons '() '())
        (git-diff/hunk-old-lines->virtual+spans
          bid old-lines new-lines new-start (min old-count new-count)
          (git-diff/hunk-anchor new-start) tab-width))))

;;; `hunks → (set-virtual-lines! …)` + `(set-extra-highlights! …)`. Two
;;; setter calls, not this repo's usual one — a single word-diff pass
;;; inherently produces two decoration kinds (old-side virtual rows,
;;; new-side highlight spans), and splitting this into two renderers to
;;; keep one setter each would call `diff-words` twice for no benefit.
(define (git-diff/render-inline! bid hunks)
  (let* ([tab-width (get-option bid "tab-width")]
         [results (map (lambda (h) (git-diff/hunk-inline-data bid h tab-width)) hunks)]
         [virtual-lines (apply append (map car results))]
         [spans (apply append (map cdr results))])
    (set-virtual-lines! git-diff/*source* bid virtual-lines)
    (set-extra-highlights! git-diff/*source* bid spans)))

;; ── Row background tint ──────────────────────────────────────────────────────

;;; One hunk -> a list of `(line scope)` line-background entries, one per
;;; new-side line: pure add -> `diff.plus`, change -> `diff.delta`. A pure
;;; delete contributes nothing — there's no live new-side line to tint,
;;; `render-inline!`'s virtual rows already cover the removed content.
(define (git-diff/hunk->line-bgs hunk)
  (let* ([old-count (list-ref hunk 1)]
         [new-start (list-ref hunk 2)]
         [new-count (list-ref hunk 3)])
    (if (= new-count 0)
        '()
        (let ([scope (if (= old-count 0) "diff.plus" "diff.delta")])
          (map (lambda (line) (list line scope)) (range new-start (+ new-start new-count)))))))

;;; `hunks → (set-line-backgrounds! …)`, the one setter call this rendering
;;; makes. No priority field on this setter (unlike `set-signs!`) — this
;;; plugin is the only tint producer for its own scopes, so there's nothing
;;; to arbitrate.
(define (git-diff/render-line-bgs! bid hunks)
  (set-line-backgrounds! git-diff/*source* bid (apply append (map git-diff/hunk->line-bgs hunks))))

;; ── Flag → renderer dispatch ────────────────────────────────────────────────────

;;; The one place a flag key maps to its renderers — every caller that needs
;;; to paint or clear a rendering (`diff.scm`'s `apply-hunks!` on a live
;;; refresh, `plugin.scm`'s toggle command on enable/disable) goes through
;;; here, so "inline? means render-inline! and render-line-bgs!" is stated
;;; once, not once per caller.
(define (git-diff/render-for! key bid hunks)
  (if (equal? key "signs?")
      (git-diff/render-signs! bid hunks)
      (begin (git-diff/render-inline! bid hunks)
             (git-diff/render-line-bgs! bid hunks))))
