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

;;; One `diff-buffer-lines` hunk -> a list of `(line text scope)` sign entries — see docs/rendering.md.
(define (git-diff/hunk->signs hunk)
  (let* ([old-count (list-ref hunk 1)]
         [new-start (list-ref hunk 2)]
         [new-count (list-ref hunk 3)])
    (cond
      [(and (= new-count 0) (= new-start 0))
       (list (list 0 "▔" "diff.minus"))]
      [(= new-count 0)
       (list (list (- new-start 1) "▁" "diff.minus"))]
      [(= old-count 0) (git-diff/line-signs new-start new-count "+" "diff.plus")]
      [else (git-diff/line-signs new-start new-count "~" "diff.delta")])))

;;; Registers first regardless of `hunks` — see docs/rendering.md's "Signs".
(define (git-diff/render-signs! pane hunks)
  (register-sign-source! git-diff/*source* pane git-diff/*sign-priority*)
  (set-signs! git-diff/*source* pane (apply append (map git-diff/hunk->signs hunks))))

;; ── Inline: deleted lines + word highlights ─────────────────────────────────
;; See docs/rendering.md.

(define (git-diff/hunk-anchor new-start)
  (if (= new-start 0)
      (cons 'before 0)
      (cons 'after (- new-start 1))))

;;; Symbol keys, not strings — `set-virtual-lines!` looks each field up as
;;; `(SteelVal::SymbolV k)`.
(define (git-diff/virtual-line-hash text anchor segments)
  (let ([base (hash 'line (cdr anchor) 'text text 'anchor (car anchor) 'scope "diff.minus.line")])
    (if (null? segments) base (hash-insert base 'segments segments))))

(define (git-diff/plain-virtual-line old-line anchor)
  (git-diff/virtual-line-hash old-line anchor '()))

(define (git-diff/virtual-line-with-segments old-line anchor word-hunks)
  (let* ([removals (filter (lambda (wh) (< (list-ref wh 0) (list-ref wh 1))) word-hunks)]
         [segments (map (lambda (wh) (list (list-ref wh 0) (list-ref wh 1) "diff.minus.word"))
                        removals)])
    (git-diff/virtual-line-hash old-line anchor segments)))

;;; `(start end scope)` triples in *buffer* char offsets — see docs/rendering.md.
(define (git-diff/word-hunks->new-side-spans line-offset word-hunks)
  (let ([additions (filter (lambda (wh) (< (list-ref wh 2) (list-ref wh 3))) word-hunks)])
    (map (lambda (wh)
           (list (+ line-offset (list-ref wh 2))
                 (+ line-offset (list-ref wh 3))
                 "diff.plus.word"))
         additions)))

;;; Char offset where each of the first `paired-count` `new-lines` starts — see docs/rendering.md.
(define (git-diff/paired-line-offsets pane new-start new-lines paired-count)
  (let ([base (line->offset pane new-start)])
    (let loop ([i 0] [offset base] [lines new-lines] [acc '()])
      (if (= i paired-count)
          (reverse acc)
          (loop (+ i 1) (+ offset (string-length (car lines)) 1) (cdr lines) (cons offset acc))))))

;;; One paired (old-line . new-line) -> `(virtual-line . spans)` — see docs/rendering.md.
(define (git-diff/paired-line->vl+spans old-line new-line line-offset anchor)
  (let* ([result (diff-words old-line new-line)]
         [word-hunks (car result)]
         [deadline-hit? (cdr result)])
    (if deadline-hit?
        (cons (git-diff/plain-virtual-line old-line anchor) '())
        (cons (git-diff/virtual-line-with-segments old-line anchor word-hunks)
              (git-diff/word-hunks->new-side-spans line-offset word-hunks)))))

;;; One hunk's removed old-side lines -> `(virtual-lines . spans)` — see docs/rendering.md
;;; for the paired/unpaired split and why `all` guards the empty-`paired` case.
(define (git-diff/hunk-old-lines->virtual+spans pane old-lines new-lines new-start paired-count anchor)
  (let* ([offsets (if (> paired-count 0)
                       (git-diff/paired-line-offsets pane new-start new-lines paired-count)
                       '())]
         ;; `cdr`, not `list-ref` by index — see docs/rendering.md.
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
         [all (if (null? paired) unpaired (append paired unpaired))])
    (cons (map car all) (apply append (map cdr all)))))

;;; One hunk -> `(virtual-lines . spans)` for `render-inline!` — see docs/rendering.md.
(define (git-diff/hunk-inline-data pane hunk)
  (let* ([old-count (list-ref hunk 1)]
         [new-start (list-ref hunk 2)]
         [new-count (list-ref hunk 3)]
         [old-lines (list-ref hunk 4)]
         [new-lines (list-ref hunk 5)])
    (if (= old-count 0)
        (cons '() '())
        (git-diff/hunk-old-lines->virtual+spans
          pane old-lines new-lines new-start (min old-count new-count)
          (git-diff/hunk-anchor new-start)))))

;;; Two setter calls, not one — see docs/rendering.md.
(define (git-diff/render-inline! pane hunks)
  (let* ([results (map (lambda (h) (git-diff/hunk-inline-data pane h)) hunks)]
         [virtual-lines (apply append (map car results))]
         [spans (apply append (map cdr results))])
    (set-virtual-lines! git-diff/*source* pane virtual-lines)
    (set-extra-highlights! git-diff/*source* pane spans)))

;; ── Line background tint ─────────────────────────────────────────────────────

(define (git-diff/hunk->line-bgs hunk)
  (let* ([old-count (list-ref hunk 1)]
         [new-start (list-ref hunk 2)]
         [new-count (list-ref hunk 3)])
    (if (= new-count 0)
        '()
        (let ([scope (if (= old-count 0) "diff.plus.line" "diff.delta.line")])
          (map (lambda (line) (list line scope)) (range new-start (+ new-start new-count)))))))

(define (git-diff/render-line-bgs! pane hunks)
  (set-line-backgrounds! git-diff/*source* pane (apply append (map git-diff/hunk->line-bgs hunks))))

;; ── Flag → renderer dispatch ────────────────────────────────────────────────────
;; See docs/rendering.md.

(define (git-diff/render-for! key pane hunks)
  (if (equal? key "signs?")
      (git-diff/render-signs! pane hunks)
      (begin (git-diff/render-inline! pane hunks)
             (git-diff/render-line-bgs! pane hunks))))
