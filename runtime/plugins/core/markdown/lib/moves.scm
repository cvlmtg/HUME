;;; core:markdown — moves.scm. See docs/tables.md.

(require "cells.scm")

(provide md-moves/blocks md-moves/row-edits md-moves/cell-edits)

(define (md-moves/edit start end text) (hash 'start-char start 'end-char end 'text text))

(define (md-moves/insert at text) (md-moves/edit at at text))

;;; The runs of consecutive numbers in `numbers`, sorted and distinct, as `(first . last)` pairs.
(define (md-moves/blocks numbers)
  (foldr (lambda (n runs)
           (if (and (pair? runs) (= (car (car runs)) (+ n 1)))
               (cons (cons n (cdr (car runs))) (cdr runs))
               (cons (cons n n) runs)))
         '()
         numbers))

(define (md-moves/line-break info line)
  (let ([at (info line)]) (+ (car at) (string-length (cdr at)))))

;;; The edits that move each `(first . last)` block of lines one line down, or up, by moving the line next to it
;;; across. `(info line)` is `(start-char . text)`. See docs/tables.md's "Moving rows and columns".
(define (md-moves/row-edits blocks down? info)
  (call! "stdlib/concat"
    (map (lambda (block)
           (let ([first (car block)] [last (cdr block)])
             (if down?
                 (let ([below (+ last 1)])
                   (list (md-moves/insert (car (info first)) (string-append (cdr (info below)) "\n"))
                         (md-moves/edit (md-moves/line-break info last) (md-moves/line-break info below) "")))
                 (let ([above (- first 1)])
                   (list (md-moves/edit (car (info above)) (car (info first)) "")
                         (md-moves/insert (+ (md-moves/line-break info last) 1)
                                          (string-append (cdr (info above)) "\n")))))))
         blocks)))

;;; The edits that move each `(first . last)` block of cell indices of the row `text`, `cells` being its cells and
;;; `start` its start char, one cell right, or left, by moving the cell next to it across.
(define (md-moves/cell-edits text start cells blocks right?)
  (let ([at (lambda (col) (+ start col))]
        [cell (lambda (index) (list-ref cells index))]
        [text-of (lambda (c) (substring text (md-cells/text-start c) (md-cells/text-end c)))])
    (call! "stdlib/concat"
      (map (lambda (block)
             (let ([first (cell (car block))] [last (cell (cdr block))])
               (if right?
                   (let ([next (cell (+ (cdr block) 1))])
                     (list (if (md-cells/pipe-before first)
                               (md-moves/insert (at (md-cells/pipe-before first)) (string-append "|" (text-of next)))
                               (md-moves/insert (at (md-cells/text-start first)) (string-append (text-of next) "|")))
                           (md-moves/edit (at (md-cells/pipe-before next)) (at (md-cells/text-end next)) "")))
                   (let ([before (cell (- (car block) 1))])
                     (list (if (md-cells/pipe-before before)
                               (md-moves/edit (at (md-cells/pipe-before before)) (at (md-cells/pipe-before first)) "")
                               (md-moves/edit (at (md-cells/text-start before)) (at (md-cells/text-start first)) ""))
                           (md-moves/insert (at (md-cells/text-end last)) (string-append "|" (text-of before))))))))
           blocks))))
