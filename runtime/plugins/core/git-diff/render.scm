;;; core:git-diff — render.scm. See docs/rendering.md.

(provide git-diff/render-signs! git-diff/render-diff! git-diff/render-for!)

(define git-diff/*source* "git-diff")

;; ── Signs ────────────────────────────────────────────────────────────────────

;;; See docs/rendering.md for the gutter-slot ordering this priority buys.
(define git-diff/*sign-priority* 0)

(define (git-diff/line-signs new-start new-count text scope)
  (map (lambda (line) (hash 'line line 'text text 'scope scope))
       (range new-start (+ new-start new-count))))

;;; One `diff-buffer-lines` hunk -> a list of `(hash 'line 'text 'scope)` sign entries — see docs/rendering.md.
(define (git-diff/hunk->signs hunk)
  (let* ([old-count (hash-ref hunk 'old-count)]
         [new-start (hash-ref hunk 'new-start)]
         [new-count (hash-ref hunk 'new-count)])
    (cond
      [(and (= new-count 0) (= new-start 0))
       (list (hash 'line 0 'text "▔" 'scope "diff.minus"))]
      [(= new-count 0)
       (list (hash 'line (- new-start 1) 'text "▁" 'scope "diff.minus"))]
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

(define (git-diff/virtual-line-hash text anchor segments)
  (let ([base (hash 'line (cdr anchor) 'text text 'anchor (car anchor) 'scope "diff.minus.line")])
    (if (null? segments) base (hash-insert base 'segments segments))))

(define (git-diff/plain-virtual-line old-line anchor)
  (git-diff/virtual-line-hash old-line anchor '()))

(define (git-diff/virtual-line-with-segments old-line anchor word-hunks)
  (let* ([removals (filter (lambda (wh) (< (hash-ref wh 'old-start) (hash-ref wh 'old-end))) word-hunks)]
         [segments (map (lambda (wh) (hash 'start (hash-ref wh 'old-start) 'end (hash-ref wh 'old-end) 'scope "diff.minus.word"))
                        removals)])
    (git-diff/virtual-line-hash old-line anchor segments)))

;;; `(hash 'start 'end 'scope)` spans in *buffer* char offsets — see docs/rendering.md.
(define (git-diff/word-hunks->new-side-spans line-offset word-hunks)
  (let ([additions (filter (lambda (wh) (< (hash-ref wh 'new-start) (hash-ref wh 'new-end))) word-hunks)])
    (map (lambda (wh)
           (hash 'start (+ line-offset (hash-ref wh 'new-start))
                 'end (+ line-offset (hash-ref wh 'new-end))
                 'scope "diff.plus.word"))
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
         [word-hunks (hash-ref result 'hunks)]
         [deadline-hit? (hash-ref result 'deadline-hit)])
    (if deadline-hit?
        (cons (git-diff/plain-virtual-line old-line anchor) '())
        (cons (git-diff/virtual-line-with-segments old-line anchor word-hunks)
              (git-diff/word-hunks->new-side-spans line-offset word-hunks)))))

;;; One hunk's removed old-side lines -> `(virtual-lines . spans)` — see docs/rendering.md
;;; for the paired/unpaired split.
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
         [all (append paired unpaired)])
    (cons (map car all) (apply append (map cdr all)))))

;;; A hunk's own `'words` -> `(virtual-lines . spans)` — see docs/rendering.md's "Word spans from the hunk".
(define (git-diff/hunk-word-data pane hunk)
  (let* ([words (hash-ref hunk 'words)]
         [new-start (hash-ref hunk 'new-start)]
         [anchor (git-diff/hunk-anchor new-start)])
    (cons (git-diff/old-lines->virtual-lines (hash-ref hunk 'old-lines) (hash-ref words 'old) anchor)
          (git-diff/line-spans->new-side-spans
            pane new-start (hash-ref hunk 'new-lines) (hash-ref words 'new)))))

(define (git-diff/line-span->segment span)
  (hash 'start (hash-ref span 'start) 'end (hash-ref span 'end) 'scope "diff.minus.word"))

;;; One virtual line per old line, each carrying the `spans` that name its line — see docs/rendering.md.
(define (git-diff/old-lines->virtual-lines old-lines spans anchor)
  (let loop ([olds old-lines] [spans spans] [line 0] [acc '()])
    (if (null? olds)
        (reverse acc)
        (let split ([rest spans] [mine '()])
          (if (and (pair? rest) (= (hash-ref (car rest) 'line) line))
              (split (cdr rest) (cons (car rest) mine))
              (loop (cdr olds) rest (+ line 1)
                    (cons (git-diff/virtual-line-hash
                            (car olds) anchor (map git-diff/line-span->segment (reverse mine)))
                          acc)))))))

;;; `spans` in buffer char offsets, one walk down `new-lines` — see docs/rendering.md.
(define (git-diff/line-spans->new-side-spans pane new-start new-lines spans)
  (if (null? spans)
      '()
      (let loop ([spans spans]
                 [line 0]
                 [offsets (git-diff/paired-line-offsets pane new-start new-lines (length new-lines))]
                 [acc '()])
        (cond
          [(null? spans) (reverse acc)]
          [(< line (hash-ref (car spans) 'line))
           (loop spans (+ line 1) (cdr offsets) acc)]
          [else
           (let ([span (car spans)] [offset (car offsets)])
             (loop (cdr spans) line offsets
                   (cons (hash 'start (+ offset (hash-ref span 'start))
                               'end (+ offset (hash-ref span 'end))
                               'scope "diff.plus.word")
                         acc)))]))))

;;; One hunk -> `(virtual-lines . spans)` for `render-inline!` — see docs/rendering.md.
(define (git-diff/hunk-inline-data pane hunk)
  (let* ([old-count (hash-ref hunk 'old-count)]
         [new-start (hash-ref hunk 'new-start)]
         [new-count (hash-ref hunk 'new-count)]
         [old-lines (hash-ref hunk 'old-lines)]
         [new-lines (hash-ref hunk 'new-lines)])
    (cond
      [(= old-count 0) (cons '() '())]
      [(hash-contains? hunk 'words) (git-diff/hunk-word-data pane hunk)]
      [else
       (git-diff/hunk-old-lines->virtual+spans
         pane old-lines new-lines new-start (min old-count new-count)
         (git-diff/hunk-anchor new-start))])))

;;; Two setter calls, not one — see docs/rendering.md.
(define (git-diff/render-inline! pane source hunks)
  (let* ([results (map (lambda (h) (git-diff/hunk-inline-data pane h)) hunks)]
         [virtual-lines (apply append (map car results))]
         [spans (apply append (map cdr results))])
    (set-virtual-lines! source pane virtual-lines)
    (set-extra-highlights! source pane spans)))

;; ── Line background tint ─────────────────────────────────────────────────────

(define (git-diff/hunk->line-bgs hunk)
  (let* ([old-count (hash-ref hunk 'old-count)]
         [new-start (hash-ref hunk 'new-start)]
         [new-count (hash-ref hunk 'new-count)])
    (if (= new-count 0)
        '()
        (let ([scope (if (= old-count 0) "diff.plus.line" "diff.delta.line")])
          (map (lambda (line) (hash 'line line 'scope scope)) (range new-start (+ new-start new-count)))))))

(define (git-diff/render-line-bgs! pane source hunks)
  (set-line-backgrounds! source pane (apply append (map git-diff/hunk->line-bgs hunks))))

;;; Inline rendering under `source`; the one entry point for any plugin's hunks — see docs/rendering.md.
(define (git-diff/render-diff! pane source hunks)
  (git-diff/render-inline! pane source hunks)
  (git-diff/render-line-bgs! pane source hunks))

;; ── Flag → renderer dispatch ────────────────────────────────────────────────────
;; See docs/rendering.md.

(define (git-diff/render-for! key pane hunks)
  (if (equal? key "signs?")
      (git-diff/render-signs! pane hunks)
      (git-diff/render-diff! pane git-diff/*source* hunks)))
