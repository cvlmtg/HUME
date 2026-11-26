;;; core:markdown — styles.scm. See docs/architecture.md, "Inline styles".

(require "queries.scm")
(require "render.scm")

(provide md-styles/toggler)

(define md-styles/query
  (string-append
    "(strong_emphasis (emphasis_delimiter) @delimiter) @bold\n"
    "(emphasis (emphasis_delimiter) @delimiter) @italic\n"
    "(strikethrough (emphasis_delimiter) @delimiter) @strikethrough\n"
    "(code_span (code_span_delimiter) @delimiter) @code\n"))

;;; The `style` spans on the lines `sels` touch, each `(span . delimiters)`.
(define (md-styles/spans pane style sels)
  (let* ([line-of (md-preview/line-of pane)]
         [ranges (map (lambda (sel) (md-preview/range-span line-of (selection-start-char sel) (selection-end-char sel)))
                      sels)])
    (map (lambda (run) (cons (car run) (call! "stdlib/concat" (cdr run))))
         (filter (lambda (run) (equal? (car (car run)) style))
                 (md-preview/runs (md-preview/captures pane "markdown.inline" md-styles/query ranges))))))

;;; Whether delimiter `d` is in the second half of `node`, the span it belongs to.
(define (md-styles/closer? node d)
  (>= (* 2 (md-preview/capture-start d)) (+ (md-preview/capture-start node) (md-preview/capture-end node))))

;;; The `(end . start)` of `span`'s opening and closing delimiters.
(define (md-styles/inside span)
  (let* ([node (car span)]
         [closers (filter (lambda (d) (md-styles/closer? node d)) (cdr span))]
         [openers (filter (lambda (d) (not (md-styles/closer? node d))) (cdr span))])
    (cons (apply max (map md-preview/capture-end openers))
          (apply min (map md-preview/capture-start closers)))))

;;; `span` and each span of `spans` nested with it, whose delimiters touch its own.
(define (md-styles/nested spans span)
  (define (just-inside? inner outer)
    (let ([inside (md-styles/inside outer)])
      (and (= (md-preview/capture-start (car inner)) (car inside))
           (= (md-preview/capture-end (car inner)) (cdr inside)))))
  (let collect ([s (let up ([s span])
                     (let ([outer (findf (lambda (o) (just-inside? s o)) spans)]) (if outer (up outer) s)))])
    (let ([inner (findf (lambda (i) (just-inside? i s)) spans)])
      (cons s (if inner (collect inner) '())))))

;;; Whether `head` is on one of `spans`' closing delimiters.
(define (md-styles/on-closer? spans head)
  (findf (lambda (span)
           (findf (lambda (d)
                    (and (md-styles/closer? (car span) d)
                         (<= (md-preview/capture-start d) head)
                         (< head (md-preview/capture-end d))))
                  (cdr span)))
         spans))

;;; `#f` when the primary selection is inside no `style` span. Otherwise `(marks . step-left?)`:
;;; the delimiters to delete, as `(start . end)` pairs, of the span around every selection inside
;;; one, and whether every selection's head is on a closing delimiter.
(define (md-styles/removal pane style)
  (let* ([sels (buffer-selections pane)]
         [spans (md-styles/spans pane style sels)]
         [around (lambda (sel)
                   (let ([span (md-preview/innermost spans (selection-start-char sel) (selection-end-char sel) car)])
                     (if span (md-styles/nested spans span) '())))]
         [arounds (map around sels)])
    (and (pair? (around (call! "stdlib/primary-selection" sels)))
         (let ([found (foldl (lambda (span found)
                               (hash-insert found (md-preview/capture-start (car span)) span))
                             (hash)
                             (call! "stdlib/concat" arounds))])
           (cons (map (lambda (d) (cons (md-preview/capture-start d) (md-preview/capture-end d)))
                      (call! "stdlib/concat" (map cdr (hash-values->list found))))
                 (md-preview/all? (lambda (pair) (md-styles/on-closer? (car pair) (selection-head (cdr pair))))
                                  (map cons arounds sels)))))))

(define (md-styles/wrap! pane marker)
  (call! "trim-selection-whitespace" pane)
  (let* ([sels (buffer-selections pane)]
         [back (if (and (call! "stdlib/single-selection?" sels) (call! "stdlib/all-single-char?" sels))
                   "move-left"
                   "undo-selection-change")])
    (let loop ([chars (string->list marker)])
      (call! "surround-add" pane (car chars))
      (unless (null? (cdr chars))
        (call! back pane)
        (loop (cdr chars))))))

(define (md-styles/toggler style marker)
  (lambda (pane)
    (let ([removal (md-styles/removal pane style)])
      (if removal
          (begin
            (edit-text! pane (map (lambda (m) (md-preview/edit (car m) (cdr m) "")) (car removal)))
            (when (cdr removal) (call! "move-left" pane)))
          (md-styles/wrap! pane marker)))))
