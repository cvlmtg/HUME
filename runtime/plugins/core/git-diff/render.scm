;;; core:git-diff — render.scm (see README.md "File layout" and "How it
;;; works" → "Rendering"). Pure `hunks → decoration records` functions, one
;;; setter call each unless noted otherwise.

(provide git-diff/render-signs! git-diff/render-inline! git-diff/render-line-bgs!
         git-diff/render-for!)

;;; Feature-scoped source name for every setter this plugin calls.
(define git-diff/*source* "git-diff")

;; ── Signs ────────────────────────────────────────────────────────────────────

;;; Only one sign producer per buffer here, so relative priority among our
;;; own signs never matters.
(define git-diff/*sign-priority* 0)

;;; One sign per line in `[new-start, new-start + new-count)`.
(define (git-diff/line-signs new-start new-count text scope)
  (map (lambda (line) (list line text scope git-diff/*sign-priority*))
       (range new-start (+ new-start new-count))))

;;; One `diff-buffer-lines` hunk `(old-start old-count new-start new-count
;;; old-lines new-lines)` -> a list of `(line text scope priority)` sign
;;; entries, one per changed line (VSCode/gitsigns density, not one per
;;; hunk).
(define (git-diff/hunk->signs hunk)
  (let* ([old-count (list-ref hunk 1)]
         [new-start (list-ref hunk 2)]
         [new-count (list-ref hunk 3)])
    (cond
      ;; Pure deletion: no new-side lines to anchor on, so mark the line
      ;; above the gap instead (gitsigns' convention). `(- new-start 1)`
      ;; rather than `new-start` sidesteps an out-of-range `set-signs!`
      ;; call for a deletion at end of file; `(max 0 …)` covers a deletion
      ;; at line 0.
      [(= new-count 0)
       (list (list (max 0 (- new-start 1)) "-" "diff.minus.gutter" git-diff/*sign-priority*))]
      [(= old-count 0) (git-diff/line-signs new-start new-count "+" "diff.plus.gutter")]
      [else (git-diff/line-signs new-start new-count "~" "diff.delta.gutter")])))

;;; `(apply append …)`, not `flatten` — a sign entry is itself a list, and
;;; `flatten` would tear each one apart. An empty `hunks` clears the gutter
;;; (`set-signs!` replaces `source`'s signs wholesale), so this doubles as
;;; the clear function.
(define (git-diff/render-signs! bid hunks)
  (set-signs! git-diff/*source* bid (apply append (map git-diff/hunk->signs hunks))))

;; ── Inline: deleted lines + word highlights ─────────────────────────────────

;;; Where a hunk's removed old-side lines attach, as a `(kind . line)` pair.
;;; `'after (- new-start 1)` when a preceding line exists — renders at the
;;; same position `'before new-start` would, but stays valid when
;;; `new-start` is the buffer's content line count (a deletion at end of
;;; file, where `'before new-start` would address the phantom trailing line
;;; and raise). `'before 0` only for a deletion at the very start.
(define (git-diff/hunk-anchor new-start)
  (if (= new-start 0)
      (cons 'before 0)
      (cons 'after (- new-start 1))))

;;; Base hashmap for a removed-line virtual row. Symbol keys, not strings —
;;; `set-virtual-lines!` looks each field up as `(SteelVal::SymbolV k)`; a
;;; string key raises "hashmap key must be a symbol". `'segments` omitted
;;; when empty rather than set to `'()`, keeping the hash to what's used.
(define (git-diff/virtual-line-hash text anchor segments)
  (let ([base (hash 'line (cdr anchor) 'text text 'anchor (car anchor) 'scope "diff.minus")])
    (if (null? segments) base (hash-insert base 'segments segments))))

;;; A whole removed line with no word-level detail. `old-line` is passed
;;; straight through — `set-virtual-lines!` accepts a literal tab in
;;; `'text` and expands it itself.
(define (git-diff/plain-virtual-line old-line anchor)
  (git-diff/virtual-line-hash old-line anchor '()))

;;; `old-line`'s virtual row with word-del `'segments` built from
;;; `diff-words`' `(old-start old-end new-start new-end old-text new-text)`
;;; hunks. Filtered to `old-start < old-end` — a pure insertion has nothing
;;; to underline on this line, and a zero-width segment would raise
;;; (`set-virtual-lines!`'s `start < end` check).
(define (git-diff/virtual-line-with-segments old-line anchor word-hunks)
  (let* ([removals (filter (lambda (wh) (< (list-ref wh 0) (list-ref wh 1))) word-hunks)]
         [segments (map (lambda (wh) (list (list-ref wh 0) (list-ref wh 1) "diff.minus.word"))
                        removals)])
    (git-diff/virtual-line-hash old-line anchor segments)))

;;; `word-hunks`' new-side fields -> `(start end scope)` triples in *buffer*
;;; char offsets (`set-extra-highlights!` addresses the whole buffer, not
;;; one line). Filtered to `new-start < new-end` for the same
;;; zero-width-raises reason as the old side.
(define (git-diff/word-hunks->new-side-spans line-offset word-hunks)
  (let ([additions (filter (lambda (wh) (< (list-ref wh 2) (list-ref wh 3))) word-hunks)])
    (map (lambda (wh)
           (list (+ line-offset (list-ref wh 2))
                 (+ line-offset (list-ref wh 3))
                 "diff.plus.word"))
         additions)))

;;; Char offset where each of the first `paired-count` `new-lines` starts,
;;; without one `line->offset` host call per line — only the hunk's first
;;; new-side line needs it; every later line is exactly its predecessor's
;;; length plus one `\n` further along.
(define (git-diff/paired-line-offsets bid new-start new-lines paired-count)
  (let ([base (line->offset bid new-start)])
    (let loop ([i 0] [offset base] [lines new-lines] [acc '()])
      (if (= i paired-count)
          (reverse acc)
          (loop (+ i 1) (+ offset (string-length (car lines)) 1) (cdr lines) (cons offset acc))))))

;;; One paired (old-line . new-line) -> `(virtual-line . spans)`, one
;;; `diff-words` call shared by both — see README's "Rendering" for why
;;; this is the sanctioned two-setter exception.
(define (git-diff/paired-line->vl+spans old-line new-line line-offset anchor)
  (let* ([result (diff-words old-line new-line)]
         [word-hunks (car result)]
         [deadline-hit? (cdr result)])
    (if deadline-hit?
        (cons (git-diff/plain-virtual-line old-line anchor) '())
        (cons (git-diff/virtual-line-with-segments old-line anchor word-hunks)
              (git-diff/word-hunks->new-side-spans line-offset word-hunks)))))

;;; One hunk's removed old-side lines -> `(virtual-lines . spans)`.
;;; `old-lines[0, paired-count)` have a same-index `new-lines` counterpart
;;; to word-diff against; any remainder gets a plain whole-line row.
(define (git-diff/hunk-old-lines->virtual+spans bid old-lines new-lines new-start paired-count anchor)
  (let* ([offsets (if (> paired-count 0)
                       (git-diff/paired-line-offsets bid new-start new-lines paired-count)
                       '())]
         ;; Walks the three lists together via `cdr`, not `list-ref` by
         ;; index — Steel lists are linked, so indexing made this quadratic
         ;; in `paired-count`.
         [paired (let loop ([olds old-lines] [news new-lines] [offs offsets] [n paired-count] [acc '()])
                   (if (= n 0)
                       (reverse acc)
                       (loop (cdr olds) (cdr news) (cdr offs) (- n 1)
                             (cons (git-diff/paired-line->vl+spans
                                     (car olds) (car news) (car offs) anchor)
                                   acc))))]
         [unpaired (map (lambda (old-line)
                          (cons (git-diff/plain-virtual-line old-line anchor) '()))
                        (list-tail old-lines paired-count))]
         [all (append paired unpaired)])
    (cons (map car all) (apply append (map cdr all)))))

;;; One hunk -> `(virtual-lines . spans)` for `render-inline!`. A pure
;;; addition contributes nothing here — `render-line-bgs!` alone covers its
;;; new-side tint.
(define (git-diff/hunk-inline-data bid hunk)
  (let* ([old-count (list-ref hunk 1)]
         [new-start (list-ref hunk 2)]
         [new-count (list-ref hunk 3)]
         [old-lines (list-ref hunk 4)]
         [new-lines (list-ref hunk 5)])
    (if (= old-count 0)
        (cons '() '())
        (git-diff/hunk-old-lines->virtual+spans
          bid old-lines new-lines new-start (min old-count new-count)
          (git-diff/hunk-anchor new-start)))))

;;; `hunks → (set-virtual-lines! …)` + `(set-extra-highlights! …)` — two
;;; setter calls, see README's "Rendering" for why.
(define (git-diff/render-inline! bid hunks)
  (let* ([results (map (lambda (h) (git-diff/hunk-inline-data bid h)) hunks)]
         [virtual-lines (apply append (map car results))]
         [spans (apply append (map cdr results))])
    (set-virtual-lines! git-diff/*source* bid virtual-lines)
    (set-extra-highlights! git-diff/*source* bid spans)))

;; ── Row background tint ──────────────────────────────────────────────────────

;;; One hunk -> `(line scope)` entries, one per new-side line: pure add ->
;;; `diff.plus`, change -> `diff.delta`. A pure delete contributes nothing —
;;; `render-inline!`'s virtual rows already cover the removed content.
(define (git-diff/hunk->line-bgs hunk)
  (let* ([old-count (list-ref hunk 1)]
         [new-start (list-ref hunk 2)]
         [new-count (list-ref hunk 3)])
    (if (= new-count 0)
        '()
        (let ([scope (if (= old-count 0) "diff.plus" "diff.delta")])
          (map (lambda (line) (list line scope)) (range new-start (+ new-start new-count)))))))

;;; No priority field on this setter (unlike `set-signs!`) — this plugin is
;;; the only tint producer for its own scopes.
(define (git-diff/render-line-bgs! bid hunks)
  (set-line-backgrounds! git-diff/*source* bid (apply append (map git-diff/hunk->line-bgs hunks))))

;; ── Flag → renderer dispatch ────────────────────────────────────────────────────

;;; The one place a flag key maps to its renderers, so "inline? means
;;; render-inline! and render-line-bgs!" is stated once, not once per
;;; caller.
(define (git-diff/render-for! key bid hunks)
  (if (equal? key "signs?")
      (git-diff/render-signs! bid hunks)
      (begin (git-diff/render-inline! bid hunks)
             (git-diff/render-line-bgs! bid hunks))))
