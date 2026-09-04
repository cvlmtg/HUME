;;; core:git-diff — render.scm. See docs/rendering.md. Pure
;;; `hunks → decoration records` functions, one setter call each unless
;;; noted otherwise.

(provide git-diff/render-signs! git-diff/render-inline! git-diff/render-line-bgs!
         git-diff/render-for!)

(define git-diff/*source* "git-diff")

;; ── Signs ────────────────────────────────────────────────────────────────────

;;; See docs/rendering.md for the gutter-slot ordering this priority buys.
(define git-diff/*sign-priority* 0)

(define (git-diff/line-signs new-start new-count text scope)
  (map (lambda (line) (list line text scope))
       (range new-start (+ new-start new-count))))

;;; One `diff-buffer-lines` hunk -> a list of `(line text scope)` sign
;;; entries, one per changed line (VSCode/gitsigns density, not one per
;;; hunk) — see docs/rendering.md for the deletion-anchor math.
(define (git-diff/hunk->signs hunk)
  (let* ([old-count (list-ref hunk 1)]
         [new-start (list-ref hunk 2)]
         [new-count (list-ref hunk 3)])
    (cond
      [(= new-count 0)
       (list (list (max 0 (- new-start 1)) "-" "diff.minus.gutter"))]
      [(= old-count 0) (git-diff/line-signs new-start new-count "+" "diff.plus.gutter")]
      [else (git-diff/line-signs new-start new-count "~" "diff.delta.gutter")])))

;;; Registers first regardless of `hunks` — a config-off buffer's first
;;; `:toggle-git-signs` needs its slot claimed before `set-signs!` accepts
;;; even an empty call. See docs/rendering.md for the `apply append`
;;; choice.
(define (git-diff/render-signs! bid hunks)
  (register-sign-source! git-diff/*source* bid git-diff/*sign-priority*)
  (set-signs! git-diff/*source* bid (apply append (map git-diff/hunk->signs hunks))))

;; ── Inline: deleted lines + word highlights ─────────────────────────────────
;; See docs/rendering.md.

(define (git-diff/hunk-anchor new-start)
  (if (= new-start 0)
      (cons 'before 0)
      (cons 'after (- new-start 1))))

;;; Symbol keys, not strings — `set-virtual-lines!` looks each field up as
;;; `(SteelVal::SymbolV k)`.
(define (git-diff/virtual-line-hash text anchor segments)
  (let ([base (hash 'line (cdr anchor) 'text text 'anchor (car anchor) 'scope "diff.minus")])
    (if (null? segments) base (hash-insert base 'segments segments))))

(define (git-diff/plain-virtual-line old-line anchor)
  (git-diff/virtual-line-hash old-line anchor '()))

(define (git-diff/virtual-line-with-segments old-line anchor word-hunks)
  (let* ([removals (filter (lambda (wh) (< (list-ref wh 0) (list-ref wh 1))) word-hunks)]
         [segments (map (lambda (wh) (list (list-ref wh 0) (list-ref wh 1) "diff.minus.word"))
                        removals)])
    (git-diff/virtual-line-hash old-line anchor segments)))

;;; `(start end scope)` triples in *buffer* char offsets — see
;;; docs/rendering.md.
(define (git-diff/word-hunks->new-side-spans line-offset word-hunks)
  (let ([additions (filter (lambda (wh) (< (list-ref wh 2) (list-ref wh 3))) word-hunks)])
    (map (lambda (wh)
           (list (+ line-offset (list-ref wh 2))
                 (+ line-offset (list-ref wh 3))
                 "diff.plus.word"))
         additions)))

;;; Char offset where each of the first `paired-count` `new-lines` starts —
;;; see docs/rendering.md for why this needs only one `line->offset` call.
(define (git-diff/paired-line-offsets bid new-start new-lines paired-count)
  (let ([base (line->offset bid new-start)])
    (let loop ([i 0] [offset base] [lines new-lines] [acc '()])
      (if (= i paired-count)
          (reverse acc)
          (loop (+ i 1) (+ offset (string-length (car lines)) 1) (cdr lines) (cons offset acc))))))

;;; One paired (old-line . new-line) -> `(virtual-line . spans)`, one
;;; `diff-words` call shared by both — see docs/rendering.md.
(define (git-diff/paired-line->vl+spans old-line new-line line-offset anchor)
  (let* ([result (diff-words old-line new-line)]
         [word-hunks (car result)]
         [deadline-hit? (cdr result)])
    (if deadline-hit?
        (cons (git-diff/plain-virtual-line old-line anchor) '())
        (cons (git-diff/virtual-line-with-segments old-line anchor word-hunks)
              (git-diff/word-hunks->new-side-spans line-offset word-hunks)))))

;;; One hunk's removed old-side lines -> `(virtual-lines . spans)` — see
;;; docs/rendering.md for the paired/unpaired split.
(define (git-diff/hunk-old-lines->virtual+spans bid old-lines new-lines new-start paired-count anchor)
  (let* ([offsets (if (> paired-count 0)
                       (git-diff/paired-line-offsets bid new-start new-lines paired-count)
                       '())]
         ;; Walks via `cdr`, not `list-ref` by index — Steel lists are
         ;; linked, so indexing would make this quadratic in `paired-count`.
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
;;; addition contributes nothing here — see docs/rendering.md.
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

;;; Two setter calls, not one — see docs/rendering.md.
(define (git-diff/render-inline! bid hunks)
  (let* ([results (map (lambda (h) (git-diff/hunk-inline-data bid h)) hunks)]
         [virtual-lines (apply append (map car results))]
         [spans (apply append (map cdr results))])
    (set-virtual-lines! git-diff/*source* bid virtual-lines)
    (set-extra-highlights! git-diff/*source* bid spans)))

;; ── Row background tint ──────────────────────────────────────────────────────

(define (git-diff/hunk->line-bgs hunk)
  (let* ([old-count (list-ref hunk 1)]
         [new-start (list-ref hunk 2)]
         [new-count (list-ref hunk 3)])
    (if (= new-count 0)
        '()
        (let ([scope (if (= old-count 0) "diff.plus" "diff.delta")])
          (map (lambda (line) (list line scope)) (range new-start (+ new-start new-count)))))))

(define (git-diff/render-line-bgs! bid hunks)
  (set-line-backgrounds! git-diff/*source* bid (apply append (map git-diff/hunk->line-bgs hunks))))

;; ── Flag → renderer dispatch ────────────────────────────────────────────────────
;; See docs/rendering.md.

(define (git-diff/render-for! key bid hunks)
  (if (equal? key "signs?")
      (git-diff/render-signs! bid hunks)
      (begin (git-diff/render-inline! bid hunks)
             (git-diff/render-line-bgs! bid hunks))))
