;;; core:markdown — moves.scm. See docs/tables.md.

(require "queries.scm")
(require "cells.scm")

(provide md-moves/blocks md-moves/neighbor md-moves/row-edits md-moves/cell-edits)

;;; The runs of consecutive numbers in the sorted `numbers`, as `(first . last)` pairs.
(define (md-moves/blocks numbers)
  (foldr (lambda (n runs)
           (if (and (pair? runs) (<= (car (car runs)) (+ n 1)))
               (cons (cons n (cdr (car runs))) (cdr runs))
               (cons (cons n n) runs)))
         '()
         numbers))

;;; The row or column a `(first . last)` block moves across: the one after it when `forward?`, else the one before.
(define (md-moves/neighbor block forward?)
  (if forward? (+ (cdr block) 1) (- (car block) 1)))

;;; The edits that move each `(first . last)` block of lines one line down, or up, by moving the line next to it
;;; across. `(info line)` is `(start-char . text)`. See docs/tables.md's "Moving rows and columns".
(define (md-moves/row-edits blocks down? info)
  (foldr (lambda (block out)
           (let* ([first (car block)]
                  [last (cdr block)]
                  [other (info (md-moves/neighbor block down?))]
                  [moved (string-append (cdr other) "\n")])
             (if down?
                 (cons (md-preview/insert (car (info first)) moved)
                       (cons (md-preview/edit (md-preview/line-end (info last)) (md-preview/line-end other) "") out))
                 (cons (md-preview/edit (car other) (car (info first)) "")
                       (cons (md-preview/insert (+ (md-preview/line-end (info last)) 1) moved) out)))))
         '()
         blocks))

;;; The edits that move each `(first . last)` block of cell indices of the row `at`, `(start-char . text)` with
;;; `cells` its cells, one cell right, or left, by moving the cell next to it across.
(define (md-moves/cell-edits at cells blocks right?)
  (let* ([char (lambda (col) (+ (car at) col))]
         [cell (lambda (index) (list-ref cells index))]
         [text-of (lambda (c) (substring (cdr at) (md-cells/text-start c) (md-cells/text-end c)))])
    (foldr (lambda (block out)
             (let ([first (cell (car block))]
                   [last (cell (cdr block))]
                   [other (cell (md-moves/neighbor block right?))])
               (if right?
                   (cons (md-preview/insert (char (md-cells/around-start first))
                                          (if (md-cells/pipe-before first)
                                              (string-append "|" (text-of other))
                                              (string-append (text-of other) "|")))
                         (cons (md-preview/edit (char (md-cells/pipe-before other)) (char (md-cells/text-end other)) "")
                               out))
                   (cons (md-preview/edit (char (md-cells/around-start other))
                                        (char (if (md-cells/pipe-before other)
                                                  (md-cells/pipe-before first)
                                                  (md-cells/text-start first)))
                                        "")
                         (cons (md-preview/insert (char (md-cells/text-end last)) (string-append "|" (text-of other)))
                               out)))))
           '()
           blocks)))
