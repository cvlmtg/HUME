;;; core:markdown — plugin.scm. See docs/architecture.md.

(require "lib/state.scm")
(require "lib/render.scm")
(require "lib/text.scm")
(require "lib/links.scm")
(require "lib/paths.scm")
(require "lib/queries.scm")
(require "lib/lists.scm")
(require "lib/cells.scm")

;; ── Config ────────────────────────────────────────────────────────────────────

(define md-preview/cfg (plugin-config))
(define md-preview/enabled-default
  (call! "stdlib/config-boolean" "core:markdown" md-preview/cfg 'preview #t))
(define md-preview/rule-width
  (call! "stdlib/config-integer" "core:markdown" md-preview/cfg 'rule-width 40 1))
(define md-lists/enabled
  (call! "stdlib/config-boolean" "core:markdown" md-preview/cfg 'continue-lists #t))

;; ── Rendering ─────────────────────────────────────────────────────────────────

(define (md-preview/track! pane)
  (md-preview/track-buffer! pane md-preview/enabled-default))

(define (md-preview/render! pane)
  (md-preview/render-buffer! pane md-preview/rule-width))

(define md-preview/render-soon!
  (debounce-by 50 md-preview/render! #:key (lambda (p . _) (buffer-key p))))

(define (md-preview/warn-missing-grammars!)
  (for-each
    (lambda (name)
      (unless (language-has-grammar? name)
        (md-preview/warn-once! name
          (string-append "markdown: no " name " grammar, so its syntax shows raw;"
                         " run :plum-install-grammar " name))))
    '("markdown" "markdown.inline")))

;; ── Lifecycle ─────────────────────────────────────────────────────────────────

(define md-preview/keymap "markdown")

(register-hook! 'on-buffer-open
  (lambda (pane)
    (md-preview/warn-missing-grammars!)
    (attach-keymap! pane md-preview/keymap)))

(register-hook! 'on-viewport-change
  (lambda (pane range headers) (md-preview/render! pane)))

(register-hook! 'on-syntax-changed
  (lambda (pane)
    (md-preview/track! pane)
    (md-preview/note-tree! pane)
    (md-preview/render-soon! pane)))

(register-hook! 'on-buffer-close
  (lambda (pane)
    (md-preview/remove-buffer! pane)
    (when (buffer-live? pane)
      (md-preview/clear! pane)
      (detach-keymap! pane md-preview/keymap))))

;; ── Links ─────────────────────────────────────────────────────────────────────

(register-hook! 'on-paste
  (lambda (pane pasted previous) (md-links/paste-link pane pasted previous)))

(register-completion-source! "markdown:paths" md-paths/complete!
  #:target 'buffer #:priority 20 #:token-chars "!\"$%&'*+,-./:;=?@[]^`{|}~")

(set-completion-triggers! "markdown" "markdown:paths" (list #\( #\/ #\<))

;; ── Lists ─────────────────────────────────────────────────────────────────────

(define (md-lists/groups pane query spans)
  (md-preview/group
    (if (language-has-grammar? "markdown")
        (syntax-captures pane "markdown" query
                         #:ranges (map (lambda (span) (hash 'start-line (car span) 'end-line (cdr span)))
                                       spans))
        '())))

(define (md-lists/numbered-lists-of pane groups line-of)
  (md-lists/numbered-lists groups line-of
    (lambda (start end)
      (cons (line->char-offset pane start) (buffer-lines pane #:range (hash 'start-line start 'end-line end))))))

(define (md-lists/prefix-edits pane prefixes)
  (let ([sels (buffer-selections pane)])
    (unless (= (length sels) (length prefixes))
      (error "markdown: the selection count differs from the list prefixes planned for it"))
    (call! "stdlib/concat"
      (map (lambda (sel prefix)
             (if prefix
                 (let* ([head (selection-head sel)]
                        [start (line->char-offset pane (char-offset->line pane head))]
                        [insert (hash 'start-char head 'end-char head 'text prefix)])
                   (if (< start head)
                       (list (hash 'start-char start 'end-char head 'text "") insert)
                       (list insert)))
                 '()))
           sels
           prefixes))))

;;; See docs/lists.md's "Order of edits".
(define (md-lists/enter! pane)
  (let* ([reader (md-preview/line-reader pane)]
         [line-of (car reader)]
         [line-info (cdr reader)]
         [heads (map selection-head (buffer-selections pane))]
         [lines (map line-of heads)])
    (if (findf (lambda (line) (md-lists/may-start-prefix? (cdr (line-info line)))) lines)
        (let* ([groups (md-lists/groups pane md-lists/query (map (lambda (line) (cons line (+ line 1))) lines))]
               [items (map (lambda (item head) (and item (md-lists/past-prefix? item head) item))
                           (md-lists/items lines groups line-info)
                           heads)])
          (if (null? (filter (lambda (item) (not (and item (md-lists/empty? item)))) items))
              (edit-text! pane (map md-lists/ended-edit items))
              (let ([numbers (cdr (md-lists/renumber (md-lists/numbered-lists-of pane groups line-of)
                                                     (md-lists/events items 'after)))])
                (insert-key! pane "enter")
                (edit-text! pane (md-lists/prefix-edits pane (md-lists/prefixes items numbers))))))
        (insert-key! pane "enter"))))

;;; See docs/lists.md's "Order of edits".
(define (md-lists/open! pane below?)
  (let* ([reader (md-preview/line-reader pane)]
         [line-of (car reader)]
         [line-info (cdr reader)]
         [sources (md-lists/distinct
                    (map (lambda (sel)
                           (line-of (if below? (- (selection-end-char sel) 1) (selection-start-char sel))))
                         (buffer-selections pane)))]
         [groups (md-lists/groups pane md-lists/query (map (lambda (line) (cons line (+ line 1))) sources))]
         [items (md-lists/items sources groups line-info)]
         [numbers (cdr (md-lists/renumber (md-lists/numbered-lists-of pane groups line-of)
                                          (md-lists/events items (if below? 'after 'before))))])
    (call! (if below? "open-line-below" "open-line-above") pane)
    (edit-text! pane (md-lists/prefix-edits pane (md-lists/prefixes items numbers)))))

;;; The generation the pass's last edit made. See docs/lists.md's "Renumbering after an edit".
(define md-lists/*renumbered* (box #f))

;;; See docs/lists.md's "Renumbering after an edit".
(define (md-lists/renumber-edited! pane changes)
  (let* ([generation (buffer-generation pane)]
         [edited (if (and changes
                          (not (buffer-read-only? pane))
                          (not (equal? generation (unbox md-lists/*renumbered*))))
                     (md-lists/edited-hunks changes)
                     '())])
    (unless (null? edited)
      (let* ([reader (md-preview/line-reader pane)]
             [line-of (car reader)]
             [line-info (cdr reader)]
             [groups (md-lists/groups pane md-lists/numbered-query
                                      (md-lists/edited-spans edited (buffer-line-count pane) line-info))]
             [lists (md-lists/numbered-lists-of pane groups line-of)]
             [edits (car (md-lists/renumber lists (md-lists/edited-events lists edited)))])
        (unless (null? edits)
          (let ([shown (buffer-panes pane)])
            (edit-text! (if (pair? shown) (car shown) pane) edits #:expect-generation generation)
            (set-box! md-lists/*renumbered* (buffer-generation pane))))))))

(define-command! "markdown/enter"
  "In a Markdown list item or quote, start the next item or quote line, renumbering an ordered list; on an empty item, end the list. Elsewhere, break the line as Enter does."
  md-lists/enter!)

(define-command! "markdown/open-below"
  "Open a line below each selection as `o` does, continuing the list item or quote it is on and renumbering an ordered list."
  (lambda (pane) (md-lists/open! pane #t))
  #:repeatable #t)

(define-command! "markdown/open-above"
  "Open a line above each selection as `O` does, continuing the list item or quote it is on and renumbering an ordered list."
  (lambda (pane) (md-lists/open! pane #f))
  #:repeatable #t)

(define (md-lists/toggle-tasks! pane)
  (let* ([reader (md-preview/line-reader pane)]
         [line-of (car reader)]
         [line-info (cdr reader)]
         [sels (buffer-selections pane)]
         [span-of (lambda (sel)
                    (cons (line-of (selection-start-char sel))
                          (+ (line-of (- (selection-end-char sel) 1)) 1)))]
         [spans (map span-of sels)]
         [lines (md-lists/distinct
                  (call! "stdlib/concat" (map (lambda (span) (range (car span) (cdr span))) spans)))]
         [items (md-lists/items lines (md-lists/groups pane md-lists/query spans) line-info)]
         [primary (span-of (call! "stdlib/primary-selection" sels))]
         [primary-items (map cdr (filter (lambda (line-item)
                                           (and (>= (car line-item) (car primary))
                                                (< (car line-item) (cdr primary))))
                                         (map cons lines items)))])
    (edit-text! pane (md-lists/task-edits items primary-items))))

(define-command! "markdown/toggle-task"
  "Check or uncheck the task boxes on the lines each selection touches: when the first box on the primary selection's lines, or else the first box of all, is checked, uncheck them all, and otherwise check them all."
  md-lists/toggle-tasks!
  #:repeatable #t)

(bind-key! 'normal "G X" "markdown/toggle-task" #:keymap md-preview/keymap)

;; ── Tables ────────────────────────────────────────────────────────────────────

;;; A reader of the lines of `pane`'s text: `(info line)` is `(start-char . text)`, and `(row-at line)` the line's
;;; row, `#f` for a line with no row or past the text.
(define (md-tables/reader pane)
  (let ([info (cdr (md-preview/line-reader pane))]
        [count (buffer-line-count pane)]
        [rows (box (hash))])
    (cons info
          (lambda (line)
            (and (<= 0 line)
                 (< line count)
                 (if (hash-contains? (unbox rows) line)
                     (hash-ref (unbox rows) line)
                     (let ([row (md-cells/row (cdr (info line)))])
                       (md-preview/box-insert! rows line row)
                       row)))))))

;;; The cell of a table under offset `head` as `(table (line . index))`, or `#f`. `tables` is a box of the tables
;;; found so far, so that heads in one table look it up once.
(define (md-tables/locate pane reader tables head)
  (let* ([line (char-offset->line pane head)]
         [row ((cdr reader) line)]
         [table (and row
                     (or (findf (lambda (t) (<= (md-cells/table-header t) line (md-cells/table-last t)))
                                (unbox tables))
                         (let ([found (md-cells/table-at (cdr reader) line)])
                           (when found (set-box! tables (cons found (unbox tables))))
                           found)))]
         [index (and table (md-cells/index-at row (- head (car ((car reader) line)))))])
    (and index (list table (cons line index)))))

(define (md-tables/cell reader target)
  (list-ref (md-cells/cells ((cdr reader) (car target))) (cdr target)))

;;; The `(start . end)` chars of `target`, a `(line . index)` cell, as the `m a |` pattern finds it when
;;; `around?`, else as the `m i |` one does.
(define (md-tables/range reader target around?)
  (let ([start (car ((car reader) (car target)))]
        [cell (md-tables/cell reader target)])
    (cons (+ start (if around? (md-cells/around-start cell) (md-cells/text-start cell)))
          (+ start (md-cells/text-end cell)))))

(define (md-tables/has-cell? reader line index)
  (< index (length (md-cells/cells ((cdr reader) line)))))

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
;;; "Dropping selections".
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

;;; For `m a |`, `cells`, `(line . index)` cells, with the delimiter cell and the empty cells of each column of
;;; `found`, `(table cell)` pairs, whose header and body cells with text `cells` all hold. See
;;; docs/tables.md's "Cells and columns".
(define (md-tables/with-whole-columns reader found cells)
  (let ([held (list->hashset cells)]
        [columns (md-tables/distinct-by (lambda (f) (cons (md-cells/table-header (car f)) (cdr (cadr f)))) found)])
    (foldr (lambda (f targets)
             (let* ([table (car f)]
                    [index (cdr (cadr f))]
                    [column (md-tables/column reader table index)]
                    [empty-cell? (lambda (cell) (md-tables/empty-cell? reader cell))]
                    [delimiter (cons (md-cells/table-delimiter table) index)])
               (if (and (md-tables/has-cell? reader (car delimiter) index)
                        (not (findf (lambda (cell) (and (not (empty-cell? cell)) (not (hashset-contains? held cell))))
                                    column)))
                   (cons delimiter (foldr cons targets (filter empty-cell? column)))
                   targets)))
           cells
           columns)))

;;; The items of `items` whose `key` no item before them has.
(define (md-tables/distinct-by key items)
  (reverse (cdr (foldl (lambda (item acc)
                         (if (hash-contains? (car acc) (key item))
                             acc
                             (cons (hash-insert (car acc) (key item) #t) (cons item (cdr acc)))))
                       (cons (hash) '())
                       items))))

(define (md-tables/by-position a b)
  (or (< (car a) (car b)) (and (= (car a) (car b)) (< (cdr a) (cdr b)))))

(define (md-tables/select-cells! pane around?)
  (let* ([reader (md-tables/reader pane)]
         [tables (box '())]
         [sels (buffer-selections pane)]
         [located (map (lambda (sel) (md-tables/locate pane reader tables (selection-head sel))) sels)]
         [selectable (filter (lambda (f) (and f (not (md-tables/empty-cell? reader (cadr f))))) located)]
         [cells (map cadr selectable)]
         [targets (sort (md-tables/distinct-by (lambda (cell) cell)
                                               (if around? (md-tables/with-whole-columns reader selectable cells) cells))
                        md-tables/by-position)]
         [primary (list-ref located (md-tables/index-where selection-primary? sels))])
    (when (pair? targets)
      (md-tables/select! pane reader (if around? "inner-paragraph" "select-line") targets
                         (and primary (cadr primary)) around?))))

(define (md-tables/select-column! pane)
  (let* ([reader (md-tables/reader pane)]
         [found (md-tables/locate pane reader (box '())
                                  (selection-head (call! "stdlib/primary-selection" (buffer-selections pane))))])
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

(define-command! "markdown/select-cell"
  "Select the text of the table cell under each selection, trimmed of blanks; a blank cell gives a cursor in it. Selections outside a table are dropped."
  (lambda (pane) (md-tables/select-cells! pane #f)))

(define-command! "markdown/select-around-cell"
  "Select the table cell under each selection with the pipe before it and its blanks, so deleting it removes the cell from its row. When the selections hold every header and body cell with text of a column, the column's delimiter cell and the pipe before each of its empty cells are selected too. Selections outside a table are dropped."
  (lambda (pane) (md-tables/select-cells! pane #t)))

(define-command! "markdown/select-column"
  "Select the text of every cell in the table column under the primary selection, one selection per row, the header included and the delimiter row left out."
  md-tables/select-column!)

(bind-key! 'normal "m i |" "markdown/select-cell" #:keymap md-preview/keymap)
(bind-key! 'normal "m a |" "markdown/select-around-cell" #:keymap md-preview/keymap)
(bind-key! 'normal "m |" "markdown/select-column" #:keymap md-preview/keymap)

(when md-lists/enabled
  (register-hook! 'on-text-changed
    (lambda (pane changes) (md-lists/renumber-edited! pane changes)))
  (bind-key! 'insert "enter" "markdown/enter" #:keymap md-preview/keymap)
  (bind-key! 'normal "o" "markdown/open-below" #:keymap md-preview/keymap)
  (bind-key! 'normal "O" "markdown/open-above" #:keymap md-preview/keymap))

;; ── Commands ──────────────────────────────────────────────────────────────────

(define (md-preview/toggle! pane)
  (unless (member (buffer-key pane) (map buffer-key (buffers)))
    (error "markdown: the current buffer is not a Markdown buffer"))
  (md-preview/track! pane)
  (let ([on? (md-preview/toggle-enabled! pane)])
    (if on?
        (md-preview/render! pane)
        (md-preview/clear! pane))
    (log! 'info (if on? "markdown preview on" "markdown preview off"))))

(define md-preview/toggle-doc
  "Toggle the Markdown live preview for the current buffer, which draws Markdown without its syntax.")

(define-command! "markdown/toggle-preview" md-preview/toggle-doc md-preview/toggle!)

(define-typed-command! "toggle-markdown-preview" md-preview/toggle-doc md-preview/toggle!)

(define (md-preview/format-text! text parsed)
  (unless (string? text)
    (error "markdown: format-text/markdown takes a text and its parsed tree"))
  (md-preview/format-text text parsed md-preview/rule-width))

(define-command! "format-text/markdown"
  "Given a Markdown text and its parsed tree, the conceals, line backgrounds and highlights that draw the text without its syntax. The editor calls it for every Markdown popup and the completion documentation box."
  md-preview/format-text!)
