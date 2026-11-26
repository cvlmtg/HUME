;;; core:markdown — table-commands.scm. See docs/tables.md.

(require "cells.scm")
(require "lists.scm")
(require "moves.scm")
(require "queries.scm")
(require "render.scm")
(require "state.scm")

(provide md-tables/select-cells! md-tables/select-column! md-tables/move-rows! md-tables/move-columns!)

;;; A reader of the rows of a text: `lines` is its `md-preview/lines`, and `(row-at line)` the line's row, `#f` for
;;; a line with no row or past the text.
(struct md-tables/rows (lines row-at))

(define (md-tables/reader pane)
  (let* ([lines (md-preview/line-reader pane)]
         [count (buffer-line-count pane)]
         [row-of (md-preview/memo (lambda (line) (md-cells/parse-row (cdr ((md-preview/lines-info lines) line)))))])
    (md-tables/rows lines (lambda (line) (and (<= 0 line) (< line count) (row-of line))))))

;;; `reader`'s `(info line)`, `(start-char . text)`.
(define (md-tables/info reader) (md-preview/lines-info (md-tables/rows-lines reader)))
(define (md-tables/line-of reader) (md-preview/lines-line-of (md-tables/rows-lines reader)))

;;; The table that holds `line`, or `#f`. `tables` is a box of the tables found so far, so that lines of one table
;;; look it up once.
(define (md-tables/table-of reader tables line)
  (or (findf (lambda (t) (<= (md-cells/table-header t) line (md-cells/table-last t))) (unbox tables))
      (let ([found (md-cells/table-at (md-tables/rows-row-at reader) line)])
        (when found (set-box! tables (cons found (unbox tables))))
        found)))

;;; The cell of a table under offset `head` as `(table (line . index))`, or `#f`.
(define (md-tables/locate reader tables head)
  (let* ([line ((md-tables/line-of reader) head)]
         [table (md-tables/table-of reader tables line)]
         [index (and table (md-cells/index-at ((md-tables/rows-row-at reader) line)
                                              (- head (car ((md-tables/info reader) line)))))])
    (and index (list table (cons line index)))))

(define (md-tables/cell reader target)
  (list-ref (md-cells/row-cells ((md-tables/rows-row-at reader) (car target))) (cdr target)))

;;; The `(start . end)` chars of `target`, a `(line . index)` cell, as the `m a |` pattern finds it when
;;; `around?`, else as the `m i |` one does.
(define (md-tables/range reader target around?)
  (let ([start (car ((md-tables/info reader) (car target)))]
        [cell (md-tables/cell reader target)])
    (cons (+ start (if around? (md-cells/around-start cell) (md-cells/cell-text-start cell)))
          (+ start (md-cells/cell-text-end cell)))))

(define (md-tables/has-cell? reader line index)
  (< index (length (md-cells/row-cells ((md-tables/rows-row-at reader) line)))))

(define (md-tables/run-length flags)
  (let loop ([rest (cdr flags)] [n 1])
    (if (and (pair? rest) (equal? (car rest) (car flags))) (loop (cdr rest) (+ n 1)) n)))

(define (md-tables/index-where pred? items)
  (let loop ([items items] [i 0])
    (cond
      [(null? items) #f]
      [(pred? (car items)) i]
      [else (loop (cdr items) (+ i 1))])))

;;; Drops the selections whose `flags` entry is `#f`, `primary` being the primary's index. See docs/tables.md's
;;; "Selecting".
(define (md-tables/keep! pane flags primary)
  (let loop ([flags (append (list-tail flags primary) (take flags primary))])
    (when (pair? flags)
      (let ([n (md-tables/run-length flags)])
        (call! (if (car flags) "cycle-primary-forward" "remove-primary-selection") pane n)
        (loop (list-tail flags n))))))

;;; Turns the selections into one per cell of `targets`, `(line . index)` cells sorted by position, with
;;; `primary`'s cell primary, or the first target when `primary` is not one. `line-command` selects the lines to
;;; sift. See docs/tables.md's "Selecting".
(define (md-tables/select! pane reader line-command targets primary around?)
  (let ([wanted (list->hashset (map (lambda (target) (md-tables/range reader target around?)) targets))]
        [primary (if (member primary targets) primary (car targets))]
        [n (length targets)])
    (call! "collapse-selection" pane)
    (call! line-command pane)
    (call! "split-selection-on-newlines" pane)
    (call! "sift-within" pane (if around? md-cells/around-pattern md-cells/inner-pattern))
    (let* ([sels (buffer-selections pane)]
           [flags (map (lambda (sel)
                         (hashset-contains? wanted (cons (selection-start-char sel) (selection-end-char sel))))
                       sels)]
           [from (md-tables/index-where selection-primary? sels)]
           [kept-before (length (filter (lambda (flag) flag) (take flags from)))])
      (unless (= n (length (filter (lambda (flag) flag) flags)))
        (error "markdown: sift-within did not find every table cell"))
      (md-tables/keep! pane flags from)
      (let ([delta (modulo (- (md-tables/index-where (lambda (t) (equal? t primary)) targets) kept-before) n)])
        (when (> delta 0) (call! "cycle-primary-forward" pane delta)))
      (unless around? (call! "trim-selection-whitespace" pane)))))

(define (md-tables/empty-cell? reader cell)
  (md-cells/empty? (md-tables/cell reader cell)))

;;; The cells at `index` of `table`'s header and body rows that have that many cells.
(define (md-tables/column reader table index)
  (map (lambda (line) (cons line index))
       (filter (lambda (line) (md-tables/has-cell? reader line index)) (md-cells/table-rows table))))

;;; For `m a |`, the `(line . index)` cells of `found`, `(table cell)` pairs, with the delimiter cell and the
;;; empty cells of each column whose header and body cells with text `found` all holds. See docs/tables.md's
;;; "Whole columns".
(define (md-tables/with-whole-columns reader found)
  (let* ([cells (map cadr found)]
         [held (list->hashset cells)]
         [columns (filter (md-tables/first-of (lambda (f) (cons (md-cells/table-header (car f)) (cdr (cadr f)))))
                          found)])
    (foldr (lambda (f targets)
             (let* ([table (car f)]
                    [index (cdr (cadr f))]
                    [column (md-tables/column reader table index)]
                    [empty-cell? (lambda (cell) (md-tables/empty-cell? reader cell))]
                    [delimiter (cons (md-cells/table-delimiter table) index)])
               (if (and (md-tables/has-cell? reader (car delimiter) index)
                        (md-preview/all? (lambda (cell) (or (empty-cell? cell) (hashset-contains? held cell))) column))
                   (cons delimiter (foldr cons targets (filter empty-cell? column)))
                   targets)))
           cells
           columns)))

;;; A predicate true of an item when no item it was asked about before had its `key`.
(define (md-tables/first-of key)
  (let ([seen (box (hash))])
    (lambda (item)
      (let ([k (key item)])
        (and (not (hash-contains? (unbox seen) k))
             (begin (md-preview/box-insert! seen k #t) #t))))))

(define (md-tables/by-position a b)
  (or (< (car a) (car b)) (and (= (car a) (car b)) (< (cdr a) (cdr b)))))

(define (md-tables/select-cells! pane around?)
  (let* ([reader (md-tables/reader pane)]
         [tables (box '())]
         [sels (buffer-selections pane)]
         [located (map (lambda (sel) (md-tables/locate reader tables (selection-head sel))) sels)]
         [selectable (filter (lambda (f) (and f (not (md-tables/empty-cell? reader (cadr f))))) located)]
         [targets (md-lists/distinct
                    (sort (if around? (md-tables/with-whole-columns reader selectable) (map cadr selectable))
                          md-tables/by-position))]
         [from (md-tables/index-where selection-primary? sels)]
         [primary (list-ref located from)])
    (when (pair? targets)
      (when around?
        (let ([first-in-table? (md-tables/first-of (lambda (f) (md-cells/table-header (car f))))])
          (md-tables/keep! pane (map (lambda (f) (and f (first-in-table? f))) located) from)))
      (md-tables/select! pane reader (if around? "inner-paragraph" "select-line") targets
                         (and primary (cadr primary)) around?))))

(define (md-tables/select-column! pane)
  (let* ([reader (md-tables/reader pane)]
         [found (md-tables/locate reader (box '())
                                  (call! "stdlib/cursor-char-offset" (buffer-selections pane)))])
    (when found
      (let* ([own (cadr found)]
             [targets (filter (lambda (cell) (not (md-tables/empty-cell? reader cell)))
                              (md-tables/column reader (car found) (cdr own)))]
             [skipped (- (length (md-cells/table-rows (car found))) (length targets))])
        (when (pair? targets)
          (call! "keep-primary-selection" pane)
          (md-tables/select! pane reader "inner-paragraph" targets own #f))
        (when (> skipped 0)
          (log! 'warning (string-append "markdown: skipped " (number->string skipped)
                                        (if (= skipped 1) " row" " rows")
                                        " with no selectable cell in this column")))))))

;;; The blocks of rows or columns the selections touch in each table, as `(table . blocks)`. `(touched line from to)`
;;; lists the rows or columns a selection touches on table line `line` when it covers columns `[from, to)` of it.
(define (md-tables/moves pane reader touched)
  (let ([info (md-tables/info reader)]
        [line-of (md-tables/line-of reader)]
        [tables (box '())])
    (map (lambda (touches) (cons (car (car touches)) (md-moves/blocks (sort (map cdr touches) <))))
         (hash-values->list
           (md-preview/group-by
             car
             (foldr (lambda (sel out)
                      (let* ([start (selection-start-char sel)]
                             [end (selection-end-char sel)]
                             [lines (md-preview/lines-of line-of start end)]
                             [bottom (- (cdr lines) 1)])
                        (foldr (lambda (line out)
                                 (let ([table (md-tables/table-of reader tables line)]
                                       [at (info line)])
                                   (if table
                                       (foldr (lambda (n out) (cons (cons table n) out))
                                              out
                                              (touched line
                                                       (if (= line (car lines)) (- start (car at)) 0)
                                                       (if (= line bottom)
                                                           (- end (car at))
                                                           (+ (string-length (cdr at)) 1))))
                                       out)))
                               out
                               (range (car lines) (cdr lines)))))
                    '()
                    (buffer-selections pane)))))))

(define (md-tables/move-rows! pane down?)
  (let* ([reader (md-tables/reader pane)]
         [moves (md-tables/moves pane reader (lambda (line from to) (list line)))]
         [movable? (lambda (move)
                     (let ([delimiter (md-cells/table-delimiter (car move))])
                       (and (> (car (car (cdr move))) delimiter)
                            (md-preview/all? (lambda (block)
                                               (< delimiter
                                                  (md-moves/neighbor block down?)
                                                  (+ (md-cells/table-last (car move)) 1)))
                                             (cdr move)))))])
    (when (and (pair? moves) (md-preview/all? movable? moves))
      (edit-text! pane (call! "stdlib/concat"
                         (map (lambda (move) (md-moves/row-edits (cdr move) down? (md-tables/info reader))) moves))))))

;;; The cells of table line `line` whose text overlaps columns `[from, to)`, or else the cell under `from`. See
;;; docs/tables.md's "Moving rows and columns".
(define (md-tables/covered-cells reader line from to)
  (let* ([row ((md-tables/rows-row-at reader) line)]
         [cells (md-cells/row-cells row)]
         [touched (map car (filter (lambda (indexed)
                                     (and (< (md-cells/cell-text-start (cdr indexed)) to)
                                          (< from (md-cells/cell-text-end (cdr indexed)))))
                                   (map cons (range 0 (length cells)) cells)))])
    (if (pair? touched)
        touched
        (let ([under (md-cells/index-at row from)]) (if under (list under) '())))))

(define (md-tables/move-columns! pane right?)
  (let* ([reader (md-tables/reader pane)]
         [moves (md-tables/moves pane reader (lambda (line from to) (md-tables/covered-cells reader line from to)))]
         [movable? (lambda (move)
                     (md-preview/all? (lambda (block)
                                        (let ([other (md-moves/neighbor block right?)])
                                          (if right?
                                              (md-tables/has-cell? reader (md-cells/table-delimiter (car move)) other)
                                              (>= other 0))))
                                      (cdr move)))]
         [short-rows (lambda (move)
                       (let* ([edge (last (cdr move))]
                              [far (max (cdr edge) (md-moves/neighbor edge right?))])
                         (length (filter (lambda (line) (not (md-tables/has-cell? reader line far)))
                                         (md-cells/table-lines (car move))))))])
    (when (and (pair? moves) (md-preview/all? movable? moves))
      (let ([short (apply + (map short-rows moves))])
        (if (> short 0)
            (log! 'warning (string-append "markdown: moved no column, since " (number->string short)
                                          (if (= short 1) " row has" " rows have") " too few cells"))
            (edit-text! pane
                        (call! "stdlib/concat"
                          (map (lambda (move)
                                 (call! "stdlib/concat"
                                   (map (lambda (line)
                                          (md-moves/cell-edits
                                            ((md-tables/info reader) line)
                                            (md-cells/row-cells ((md-tables/rows-row-at reader) line))
                                            (cdr move)
                                            right?))
                                        (md-cells/table-lines (car move)))))
                               moves))))))))
