;;; core:markdown-preview — table.scm. See README.md.

(require "queries.scm")

(provide md-preview/table-spans md-preview/table-elements)

(define md-preview/table-scope "punctuation.special")

(define (md-preview/table-spans tables line-of)
  (map (lambda (t) (md-preview/line-span line-of t)) tables))

(define (md-preview/char-start conceal) (hash-ref conceal 'start-char))

(define (md-preview/cell-span cell line-of line-info)
  (let ([start (md-preview/capture-start cell)])
    (cons start
          (+ start (string-length
                     (trim-end (md-preview/line-slice start (md-preview/capture-end cell)
                                                      line-of line-info)))))))

(define (md-preview/shown-width span conceals line-of line-info)
  (let loop ([cs conceals] [at (car span)] [width 0])
    (let ([raw (lambda (end) (string-width (md-preview/line-slice at end line-of line-info)))])
      (cond
        [(null? cs) (+ width (raw (cdr span)))]
        [(< (md-preview/char-start (car cs)) at) (loop (cdr cs) at width)]
        [else
         (let ([c (car cs)])
           (loop (cdr cs)
                 (hash-ref c 'end-char)
                 (+ width (raw (md-preview/char-start c))
                    (string-width (or (hash-try-get c 'text) "")))))]))))

(define (md-preview/measure-row row cells conceals line-of line-info)
  (let* ([spans (map (lambda (c) (md-preview/cell-span c line-of line-info)) cells)]
         [owned (md-preview/bucket (map (lambda (s) (cons (car s) (- (cdr s) 1))) spans)
                                   conceals
                                   md-preview/char-start)])
    (cons row
          (map (lambda (s cs) (list (car s) (cdr s) (md-preview/shown-width s cs line-of line-info)))
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

(define (md-preview/spaces n) (make-string n #\space))

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
                                             (string-append " " (md-preview/spaces (car column)) " │"))
                                           columns)))
                reveal)
              out)
        (let* ([cell (car cells)]
               [column (car columns)]
               [pads (md-preview/pads (cdr column) (- (car column) (list-ref cell 2)))])
          (loop (list-ref cell 1)
                (cdr cells)
                (cdr columns)
                (string-append (md-preview/spaces (cdr pads)) " ")
                (cons (md-preview/table-conceal from (car cell)
                                                (string-append lead "│ " (md-preview/spaces (car pads)))
                                                reveal)
                      out))))))

(define (md-preview/layout table rows delimiters conceals line-of line-info)
  (let* ([reveal (md-preview/line-span line-of table)]
         [measured (map (lambda (r cs) (md-preview/measure-row (car r) (cdr r) cs line-of line-info))
                        rows
                        (md-preview/bucket (map (lambda (r) (md-preview/extent (car r))) rows)
                                           conceals
                                           md-preview/char-start))]
         [alignments (foldl (lambda (d found)
                              (append found
                                      (map (lambda (c) (md-preview/alignment c line-of line-info))
                                           (cdr d))))
                            '()
                            delimiters)]
         [widths (foldl (lambda (r widths)
                          (md-preview/widen widths (map (lambda (c) (list-ref c 2)) (cdr r))))
                        (map (lambda (_) 0) alignments)
                        measured)]
         [line-start (car (line-info (hash-ref reveal 'start-line)))]
         [indent (md-preview/spaces
                   (string-width (md-preview/line-slice line-start (md-preview/capture-start table)
                                                        line-of line-info)))]
         [columns (md-preview/columns widths alignments)]
         [rules (map (lambda (d)
                       (md-preview/table-conceal (md-preview/capture-start (car d))
                                                 (md-preview/capture-end (car d))
                                                 (md-preview/rule "├" "┼" "┤" widths)
                                                 reveal))
                     delimiters)])
    (hash 'reveal reveal 'widths widths 'indent indent
          'conceals (foldl (lambda (r out) (md-preview/row-conceals (car r) (cdr r) columns reveal out))
                           rules
                           measured))))

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
  (and (call! "stdlib/find"
              (lambda (r) (and (<= (hash-ref r 'start-line) line) (< line (hash-ref r 'end-line))))
              ranges)
       #t))

(define (md-preview/edges layouts ranges line-info line-count)
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
    (foldl append
           '()
           (map (lambda (t top bottom)
                  (append (md-preview/edge t top "┌" "┬" "┐" line-info)
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

(define (md-preview/table-elements groups inline ranges line-of line-info line-count)
  (let* ([tables (md-preview/captured groups "table")]
         [spans (map md-preview/extent tables)]
         [inline (sort inline md-preview/conceal-before?)]
         [record-start (lambda (r) (md-preview/capture-start (car r)))]
         [layouts (map (lambda (t rows delimiters conceals)
                         (md-preview/layout t rows delimiters conceals line-of line-info))
                       tables
                       (md-preview/bucket spans
                                          (md-preview/records (md-preview/captured groups "table-row")
                                                              (md-preview/captured groups "table-cell"))
                                          record-start)
                       (md-preview/bucket spans
                                          (md-preview/records
                                            (md-preview/captured groups "table-delimiter-row")
                                            (md-preview/captured groups "table-delimiter-cell"))
                                          record-start)
                       (md-preview/bucket spans inline md-preview/char-start))])
    (hash 'conceals (foldl (lambda (t out) (append (hash-ref t 'conceals) out))
                           (md-preview/edges layouts ranges line-info line-count)
                           layouts)
          'inline (md-preview/within-tables spans (map (lambda (t) (hash-ref t 'reveal)) layouts) inline))))
