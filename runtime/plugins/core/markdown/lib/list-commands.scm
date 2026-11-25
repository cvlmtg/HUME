;;; core:markdown — list-commands.scm. See docs/lists.md.

(require "lists.scm")
(require "queries.scm")
(require "render.scm")

(provide md-lists/enter! md-lists/open! md-lists/renumber-edited! md-lists/toggle-tasks!)

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

(define (md-lists/toggle-tasks! pane)
  (let* ([reader (md-preview/line-reader pane)]
         [line-of (car reader)]
         [line-info (cdr reader)]
         [sels (buffer-selections pane)]
         [span-of (lambda (sel) (md-preview/lines-of line-of (selection-start-char sel) (selection-end-char sel)))]
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
