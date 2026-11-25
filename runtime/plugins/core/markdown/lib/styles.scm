;;; core:markdown — styles.scm. See docs/architecture.md, "Inline styles".

(require "queries.scm")

(provide md-styles/removal)

(define md-styles/query
  (string-append
    "(strong_emphasis (emphasis_delimiter) @delimiter) @bold\n"
    "(emphasis (emphasis_delimiter) @delimiter) @italic\n"
    "(strikethrough (emphasis_delimiter) @delimiter) @strikethrough\n"
    "(code_span (code_span_delimiter) @delimiter) @code\n"))

;;; The `style` spans on the lines `sels` touch, each `(span . delimiters)`.
(define (md-styles/spans pane style sels)
  (if (language-has-grammar? "markdown.inline")
      (let* ([line-of (lambda (offset) (char-offset->line pane offset))]
             [ranges (map (lambda (sel)
                            (let ([lines (md-preview/lines-of line-of (selection-start-char sel)
                                                              (selection-end-char sel))])
                              (hash 'start-line (car lines) 'end-line (cdr lines))))
                          sels)]
             [matches (syntax-captures pane "markdown.inline" md-styles/query #:ranges ranges)])
        (map (lambda (run) (cons (car run) (call! "stdlib/concat" (cdr run))))
             (filter (lambda (run) (equal? (car (car run)) style)) (md-preview/runs matches))))
      '()))

(define (md-styles/innermost spans start end)
  (foldl (lambda (span best)
           (let ([node (car span)])
             (if (and (<= (md-preview/capture-start node) start)
                      (<= end (md-preview/capture-end node))
                      (or (not best) (md-preview/inside? node (car best))))
                 span
                 best)))
         #f
         spans))

;;; `span` and each span of `spans` wrapping it one character wider on each side.
(define (md-styles/with-outer spans span)
  (let* ([node (car span)]
         [outer (findf (lambda (o)
                         (and (= (md-preview/capture-start (car o)) (- (md-preview/capture-start node) 1))
                              (= (md-preview/capture-end (car o)) (+ (md-preview/capture-end node) 1))))
                       spans)])
    (if outer (cons span (md-styles/with-outer spans outer)) (list span))))

;;; Whether `head` is on one of `spans`' closing delimiters, those in a span's second half.
(define (md-styles/on-closer? spans head)
  (findf (lambda (span)
           (let ([node (car span)])
             (findf (lambda (d)
                      (and (>= (* 2 (md-preview/capture-start d))
                               (+ (md-preview/capture-start node) (md-preview/capture-end node)))
                           (<= (md-preview/capture-start d) head)
                           (< head (md-preview/capture-end d))))
                    (cdr span))))
         spans))

;;; `#f` when the primary selection is inside no `style` span. Otherwise `(marks . step-left?)`:
;;; the delimiters to delete, as `(start . end)` pairs, of the span around every selection inside
;;; one, and whether every selection's head is on a closing delimiter.
(define (md-styles/removal pane style)
  (let* ([sels (buffer-selections pane)]
         [spans (md-styles/spans pane style sels)]
         [around (lambda (sel)
                   (let ([span (md-styles/innermost spans (selection-start-char sel) (selection-end-char sel))])
                     (cond
                       [(not span) '()]
                       [(equal? style "strikethrough") (md-styles/with-outer spans span)]
                       [else (list span)])))]
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
