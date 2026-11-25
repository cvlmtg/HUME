;;; core:markdown — cells.scm. See docs/tables.md.

(require "queries.scm")
(require "lists.scm")

(provide md-cells/inner-pattern md-cells/around-pattern md-cells/row md-cells/cells
         md-cells/pipe-before md-cells/text-start md-cells/text-end md-cells/around-start md-cells/empty?
         md-cells/index-at md-cells/table-at md-cells/table-header md-cells/table-delimiter md-cells/table-last
         md-cells/table-lines md-cells/table-rows)

;;; `md-lists/container-of` and `md-cells/pipes` read a row as these patterns split it. See docs/tables.md's
;;; "The text of a row".
(define md-cells/prefix-pattern "^(?:[ \\t]|>|[-+*][ \\t]|[0-9]{1,9}[.)][ \\t])+")
(define md-cells/run-pattern "(?:\\\\.|[^|\\n])+")
(define md-cells/inner-pattern (string-append md-cells/prefix-pattern "|" md-cells/run-pattern))
(define md-cells/around-pattern
  (string-append md-cells/prefix-pattern "|\\|(?:\\\\.|[^|\\n])*|" md-cells/run-pattern))

;;; The columns of the unescaped pipes in `text` from `col` on.
(define (md-cells/pipes text col)
  (let loop ([col col] [pipes '()])
    (cond
      [(>= col (string-length text)) (reverse pipes)]
      [(char=? (string-ref text col) #\\) (loop (+ col 2) pipes)]
      [(char=? (string-ref text col) #\|) (loop (+ col 1) (cons col pipes))]
      [else (loop (+ col 1) pipes)])))

(define (md-cells/cell pipe-before text-start text-end) (list pipe-before text-start text-end))
(define (md-cells/pipe-before cell) (list-ref cell 0))
(define (md-cells/text-start cell) (list-ref cell 1))
(define (md-cells/text-end cell) (list-ref cell 2))

;;; Where `m a |` starts the cell: the pipe before it, or its text when it has none.
(define (md-cells/around-start cell) (or (md-cells/pipe-before cell) (md-cells/text-start cell)))

(define (md-cells/empty? cell) (= (md-cells/text-start cell) (md-cells/text-end cell)))

(define (md-cells/delimiter-text? text)
  (let* ([from (md-preview/skip-blanks text 0)]
         [from (if (and (< from (string-length text)) (char=? (string-ref text from) #\:)) (+ from 1) from)]
         [dashes (md-preview/skip-while text from (lambda (c) (char=? c #\-)))]
         [after (if (and (< dashes (string-length text)) (char=? (string-ref text dashes) #\:)) (+ dashes 1) dashes)])
    (and (> dashes from) (md-preview/blank-from? text after))))

;;; The row `text` holds as `(cells delimiter?)`, each cell `(pipe-before text-start text-end)` in columns, or `#f`
;;; for a line with no unescaped pipe.
(define (md-cells/row text)
  (let* ([prefix (md-lists/container-end (md-lists/container-of text))]
         [pipes (md-cells/pipes text prefix)])
    (and (pair? pipes)
         (let* ([closer? (md-preview/blank-from? text (+ (last pipes) 1))]
                [bounds (if closer? pipes (append pipes (list (string-length text))))]
                [opened (map (lambda (pipe end) (md-cells/cell pipe (+ pipe 1) end))
                             (take bounds (- (length bounds) 1))
                             (cdr bounds))]
                [cells (if (= (car pipes) prefix)
                           opened
                           (cons (md-cells/cell #f prefix (car pipes)) opened))])
           (list cells
                 (and (pair? cells)
                      (md-preview/all? (lambda (cell)
                                         (md-cells/delimiter-text?
                                           (substring text (md-cells/text-start cell) (md-cells/text-end cell))))
                                       cells)))))))

(define (md-cells/cells row) (list-ref row 0))
(define (md-cells/delimiter? row) (list-ref row 1))

;;; The index of the cell of `row` that holds column `col`, or `#f` for a row with no cell. See docs/tables.md's
;;; "The cell under an offset".
(define (md-cells/index-at row col)
  (let loop ([cells (md-cells/cells row)] [i 0])
    (cond
      [(null? cells) #f]
      [(or (null? (cdr cells)) (<= col (md-cells/text-end (car cells)))) i]
      [else (loop (cdr cells) (+ i 1))])))

(define (md-cells/table header delimiter last) (list header delimiter last))
(define (md-cells/table-header table) (list-ref table 0))
(define (md-cells/table-delimiter table) (list-ref table 1))
(define (md-cells/table-last table) (list-ref table 2))

(define (md-cells/table-lines table)
  (range (md-cells/table-header table) (+ (md-cells/table-last table) 1)))

;;; The header and body lines of `table`.
(define (md-cells/table-rows table)
  (filter (lambda (line) (not (= line (md-cells/table-delimiter table)))) (md-cells/table-lines table)))

;;; The table that holds `line`, or `#f`. `(row-at line)` gives a line's row, `#f` for a line with no row or
;;; past the text. See docs/tables.md's "Tables".
(define (md-cells/table-at row-at line)
  (and (row-at line)
       (let* ([top (let up ([l line]) (if (row-at (- l 1)) (up (- l 1)) l))]
              [bottom (let down ([l line]) (if (row-at (+ l 1)) (down (+ l 1)) l))]
              [delimiter (findf (lambda (l) (md-cells/delimiter? (row-at l))) (range (+ top 1) (+ bottom 1)))])
         (and delimiter
              (>= line (- delimiter 1))
              (md-cells/table (- delimiter 1) delimiter bottom)))))
