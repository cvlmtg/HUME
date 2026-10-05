;;; core:git-diff — render.scm. See docs/rendering.md.

(provide git-diff/*source* git-diff/render-signs! git-diff/render-diff!)

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

(define (git-diff/word-span start end scope)
  (hash 'start start 'end end 'scope scope))

(define (git-diff/line-span->segment span)
  (git-diff/word-span (hash-ref span 'start) (hash-ref span 'end) "diff.minus.word"))

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
                 [lines new-lines]
                 [offset (line->offset pane new-start)]
                 [acc '()])
        (cond
          [(null? spans) (reverse acc)]
          [(< line (hash-ref (car spans) 'line))
           (loop spans (+ line 1) (cdr lines) (+ offset (string-length (car lines)) 1) acc)]
          [else
           (let ([span (car spans)])
             (loop (cdr spans) line lines offset
                   (cons (git-diff/word-span (+ offset (hash-ref span 'start))
                                             (+ offset (hash-ref span 'end))
                                             "diff.plus.word")
                         acc)))]))))

;;; One hunk -> `(virtual-lines . spans)` for `render-inline!` — see docs/rendering.md.
(define (git-diff/hunk-inline-data pane hunk)
  (if (= (hash-ref hunk 'old-count) 0)
      (cons '() '())
      (let* ([words (hash-ref hunk 'words)]
             [new-start (hash-ref hunk 'new-start)]
             [anchor (git-diff/hunk-anchor new-start)])
        (cons (git-diff/old-lines->virtual-lines (hash-ref hunk 'old-lines) (hash-ref words 'old) anchor)
              (git-diff/line-spans->new-side-spans
                pane new-start (hash-ref hunk 'new-lines) (hash-ref words 'new))))))

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
