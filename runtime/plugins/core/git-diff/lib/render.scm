;;; core:git-diff — render.scm. See docs/rendering.md.

(provide git-diff/*source* git-diff/hunks->signs git-diff/render-signs! git-diff/render-diff!)

(define git-diff/*source* "git-diff")

(define (git-diff/pure-addition? hunk)
  (= (hash-ref hunk 'old-start-line) (hash-ref hunk 'old-end-line)))

(define (git-diff/pure-deletion? hunk)
  (= (hash-ref hunk 'new-start-line) (hash-ref hunk 'new-end-line)))

;;; Where a hunk's removed lines sit, as a `(kind . line)` pair — see docs/rendering.md's "Anchors".
(define (git-diff/hunk-anchor new-start)
  (if (= new-start 0)
      (cons 'before 0)
      (cons 'after (- new-start 1))))

;; ── Signs ────────────────────────────────────────────────────────────────────

;;; See docs/rendering.md for the gutter-slot ordering this priority buys.
(define git-diff/*sign-priority* 0)

(define (git-diff/line-signs new-start new-end text scope)
  (list (hash 'line new-start 'end-line new-end 'text text 'scope scope)))

;;; One `diff-buffer-lines` hunk -> a list of `set-signs!` entries — see docs/rendering.md.
(define (git-diff/hunk->signs hunk)
  (let ([new-start (hash-ref hunk 'new-start-line)]
        [new-end (hash-ref hunk 'new-end-line)])
    (cond
      [(git-diff/pure-deletion? hunk)
       (let ([anchor (git-diff/hunk-anchor new-start)])
         (list (hash 'line (cdr anchor)
                     'text (if (equal? (car anchor) 'before) "▔" "▁")
                     'scope "diff.minus")))]
      [(git-diff/pure-addition? hunk) (git-diff/line-signs new-start new-end "+" "diff.plus")]
      [else (git-diff/line-signs new-start new-end "~" "diff.delta")])))

(define (git-diff/hunks->signs hunks)
  (call! "stdlib/concat" (map git-diff/hunk->signs hunks)))

;;; Registers first regardless of `signs` — see docs/rendering.md's "Signs".
(define (git-diff/render-signs! pane signs)
  (register-sign-source! pane git-diff/*source* git-diff/*sign-priority*)
  (set-signs! pane git-diff/*source* signs))

;; ── Inline: deleted lines + word highlights ─────────────────────────────────
;; See docs/rendering.md.

(define (git-diff/virtual-line-hash text anchor segments)
  (let ([base (hash 'line (cdr anchor) 'text text 'side (car anchor) 'scope "diff.minus.line")])
    (if (null? segments) base (hash-insert base 'segments segments))))

;;; `span`'s `start-key`/`end-key` pair, with a theme scope.
(define (git-diff/word-span start-key end-key span scope)
  (hash start-key (hash-ref span start-key) end-key (hash-ref span end-key) 'scope scope))

;;; One virtual line per old line, each carrying the `spans` that name its line — see docs/rendering.md.
(define (git-diff/old-lines->virtual-lines old-lines spans anchor first-line)
  (let loop ([olds old-lines] [spans spans] [line first-line] [acc '()])
    (if (null? olds)
        (reverse acc)
        (let split ([rest spans] [mine '()])
          (if (and (pair? rest) (= (hash-ref (car rest) 'line) line))
              (split (cdr rest) (cons (car rest) mine))
              (loop (cdr olds) rest (+ line 1)
                    (cons (git-diff/virtual-line-hash
                            (car olds) anchor
                            (map (lambda (span)
                                   (git-diff/word-span 'start-char-col 'end-char-col span
                                                       "diff.minus.word"))
                                 (reverse mine)))
                          acc)))))))

;;; One hunk -> `(virtual-lines . spans)` for `render-inline!` — see docs/rendering.md.
(define (git-diff/hunk-inline-data hunk)
  (if (git-diff/pure-addition? hunk)
      (cons '() '())
      (let* ([words (or (hash-ref hunk 'words) (hash 'old '() 'new '()))]
             [anchor (git-diff/hunk-anchor (hash-ref hunk 'new-start-line))])
        (cons (git-diff/old-lines->virtual-lines
                (hash-ref hunk 'old-lines) (hash-ref words 'old) anchor (hash-ref hunk 'old-start-line))
              (map (lambda (span)
                     (git-diff/word-span 'start-char 'end-char span "diff.plus.word"))
                   (hash-ref words 'new))))))

;;; Two setter calls, not one — see docs/rendering.md.
(define (git-diff/render-inline! pane source hunks)
  (let* ([results (map git-diff/hunk-inline-data hunks)]
         [virtual-lines (call! "stdlib/concat" (map car results))]
         [spans (call! "stdlib/concat" (map cdr results))])
    (set-virtual-lines! pane source virtual-lines)
    (set-extra-highlights! pane source spans)))

;; ── Line background tint ─────────────────────────────────────────────────────

(define (git-diff/hunk->line-bgs hunk)
  (if (git-diff/pure-deletion? hunk)
      '()
      (list (hash 'line (hash-ref hunk 'new-start-line) 'end-line (hash-ref hunk 'new-end-line)
                  'scope (if (git-diff/pure-addition? hunk) "diff.plus.line" "diff.delta.line")))))

(define (git-diff/render-line-bgs! pane source hunks)
  (set-line-backgrounds! pane source (call! "stdlib/concat" (map git-diff/hunk->line-bgs hunks))))

;;; Inline rendering under `source`; the one entry point for any plugin's hunks — see docs/rendering.md.
(define (git-diff/render-diff! pane source hunks)
  (git-diff/render-inline! pane source hunks)
  (git-diff/render-line-bgs! pane source hunks))
