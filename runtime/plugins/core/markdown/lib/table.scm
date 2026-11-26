;;; core:markdown — table.scm. See docs/elements.md.

(require "queries.scm")

(provide md-preview/table-elements)

(define md-preview/table-scope "punctuation.special")

(define (md-preview/cell-span cell slice)
  (let ([start (md-preview/capture-start cell)])
    (cons start
          (+ start (string-length (trim-end (slice start (md-preview/capture-end cell))))))))

;;; A cell's `[start, end)` chars and the width its shown text takes.
(struct md-preview/measured (start end width))

(define (md-preview/measure-row row cells conceals line-of line-info)
  (let* ([info (md-preview/capture-line row line-of line-info)]
         [slice (md-preview/line-slicer info)]
         [spans (map (lambda (c) (md-preview/cell-span c slice)) cells)]
         [owned (md-preview/bucket (map (lambda (s) (cons (car s) (- (cdr s) 1))) spans)
                                   conceals
                                   md-preview/char-start)])
    (cons row
          (map (lambda (s cs) (md-preview/measured (car s) (cdr s) (string-width (md-preview/shown-text s cs slice))))
               spans
               owned))))

(define (md-preview/alignment cell line-of line-info)
  (let* ([text (md-preview/line-slice (md-preview/capture-start cell) (md-preview/capture-end cell)
                                      line-of line-info)]
         [left? (starts-with? text ":")]
         [right? (ends-with? text ":")])
    (cond
      [(and left? right?) 'center]
      [right? 'right]
      [else 'left])))

(define (md-preview/pads alignment extra)
  (cond
    [(equal? alignment 'right) (cons extra 0)]
    [(equal? alignment 'center) (let ([before (quotient extra 2)]) (cons before (- extra before)))]
    [else (cons 0 extra)]))

(define (md-preview/columns widths alignments)
  (if (null? widths)
      '()
      (cons (cons (car widths) (if (null? alignments) 'left (car alignments)))
            (md-preview/columns (cdr widths) (if (null? alignments) '() (cdr alignments))))))

(define (md-preview/widen widths row-widths)
  (cond
    [(null? widths) row-widths]
    [(null? row-widths) widths]
    [else (cons (max (car widths) (car row-widths))
                (md-preview/widen (cdr widths) (cdr row-widths)))]))

(define (md-preview/rule left middle right widths)
  (string-append left
                 (string-join (map (lambda (w) (make-string (+ w 2) #\─)) widths) middle)
                 right))

(define (md-preview/table-conceal start end text reveal)
  (md-preview/conceal start end text md-preview/table-scope reveal))

(define (md-preview/row-conceals row cells columns reveal out)
  (let loop ([from (md-preview/capture-start row)] [cells cells] [columns columns] [lead ""] [out out])
    (if (null? cells)
        (cons (md-preview/table-conceal
                from (md-preview/capture-end row)
                (string-append lead "│"
                               (apply string-append
                                      (map (lambda (column)
                                             (string-append " " (make-string (car column) #\space) " │"))
                                           columns)))
                reveal)
              out)
        (let* ([cell (car cells)]
               [column (car columns)]
               [pads (md-preview/pads (cdr column) (- (car column) (md-preview/measured-width cell)))])
          (loop (md-preview/measured-end cell)
                (cdr cells)
                (cdr columns)
                (string-append (make-string (cdr pads) #\space) " ")
                (cons (md-preview/table-conceal from (md-preview/measured-start cell)
                                                (string-append lead "│ " (make-string (car pads) #\space))
                                                reveal)
                      out))))))

(define (md-preview/layout table rows delimiters conceals line-of line-info)
  (let* ([reveal (md-preview/line-span line-of table)]
         [measured (map (lambda (r cs) (md-preview/measure-row (car r) (cdr r) cs line-of line-info))
                        rows
                        (md-preview/bucket (map (lambda (r) (md-preview/extent (car r))) rows)
                                           conceals
                                           md-preview/char-start))]
         [alignments (call! "stdlib/concat"
                       (map (lambda (d)
                              (map (lambda (c) (md-preview/alignment c line-of line-info)) (cdr d)))
                            delimiters))]
         [widths (foldl (lambda (r widths)
                          (md-preview/widen widths (map md-preview/measured-width (cdr r))))
                        (map (lambda (_) 0) alignments)
                        measured)]
         [line-start (car (line-info (hash-ref reveal 'start-line)))]
         [indent (make-string
                   (string-width (md-preview/line-slice line-start (md-preview/capture-start table)
                                                        line-of line-info))
                   #\space)]
         [columns (md-preview/columns widths alignments)]
         [rules (map (lambda (d)
                       (md-preview/table-conceal (md-preview/capture-start (car d))
                                                 (md-preview/capture-end (car d))
                                                 (md-preview/rule "├" "┼" "┤" widths)
                                                 reveal))
                     delimiters)])
    (hash 'reveal reveal 'widths widths 'indent indent
          'conceals (let loop ([rows measured] [rules rules] [out '()])
                      (cond
                        [(and (pair? rules)
                              (or (null? rows)
                                  (< (md-preview/char-start (car rules)) (md-preview/capture-start (car (car rows))))))
                         (loop rows (cdr rules) (cons (car rules) out))]
                        [(pair? rows)
                         (loop (cdr rows)
                               rules
                               (md-preview/row-conceals (car (car rows)) (cdr (car rows)) columns reveal out))]
                        [else (reverse out)])))))

(define (md-preview/empty-line? line line-info)
  (= 0 (string-length (cdr (line-info line)))))

(define (md-preview/edge layout line left middle right line-info)
  (if line
      (let ([at (car (line-info line))])
        (list (md-preview/table-conceal
                at at
                (string-append (hash-ref layout 'indent)
                               (md-preview/rule left middle right (hash-ref layout 'widths)))
                (hash-ref layout 'reveal))))
      '()))

(define (md-preview/in-ranges? line ranges)
  (findf (lambda (r) (and (<= (hash-ref r 'start-line) line) (< line (hash-ref r 'end-line)))) ranges))

;;; Each layout's conceals between the edges drawn above and below it, in position order.
(define (md-preview/framed layouts ranges line-info line-count)
  (let* ([bottoms (map (lambda (t)
                         (let ([below (hash-ref (hash-ref t 'reveal) 'end-line)])
                           (and (< below line-count) (md-preview/empty-line? below line-info) below)))
                       layouts)]
         [tops (map (lambda (t)
                      (let ([above (- (hash-ref (hash-ref t 'reveal) 'start-line) 1)])
                        (and (or (= above 0) (md-preview/in-ranges? (- above 1) ranges))
                             (md-preview/empty-line? above line-info)
                             (not (member above bottoms))
                             above)))
                    layouts)])
    (call! "stdlib/concat"
      (map (lambda (t top bottom)
             (append (md-preview/edge t top "┌" "┬" "┐" line-info)
                     (hash-ref t 'conceals)
                     (md-preview/edge t bottom "└" "┴" "┘" line-info)))
           layouts tops bottoms))))

(define (md-preview/within-tables spans reveals conceals)
  (let loop ([spans spans] [reveals reveals] [cs conceals] [out '()])
    (cond
      [(null? cs) (reverse out)]
      [(and (pair? spans) (> (md-preview/char-start (car cs)) (cdr (car spans))))
       (loop (cdr spans) (cdr reveals) cs out)]
      [(and (pair? spans) (>= (md-preview/char-start (car cs)) (car (car spans))))
       (loop spans reveals (cdr cs) (cons (hash-insert (car cs) 'reveal (car reveals)) out))]
      [else (loop spans reveals (cdr cs) (cons (car cs) out))])))

(define (md-preview/table-elements runs inline ranges line-of line-info line-count)
  (if (null? runs)
      (hash 'conceals '() 'inline inline)
      (let* ([spans (map (lambda (run) (md-preview/extent (car run))) runs)]
             [layouts (map (lambda (run conceals)
                             (let* ([kinds (md-preview/group (cdr run))]
                                    [rows (lambda (name)
                                            (map (lambda (row) (cons (car row) (md-preview/children row)))
                                                 (md-preview/runs-of kinds name)))])
                               (md-preview/layout (car run) (rows "table-row") (rows "table-delimiter-row")
                                                  conceals line-of line-info)))
                           runs
                           (md-preview/bucket spans inline md-preview/char-start))])
        (hash 'conceals (md-preview/framed layouts ranges line-info line-count)
              'inline (md-preview/within-tables spans (map (lambda (t) (hash-ref t 'reveal)) layouts) inline)))))
