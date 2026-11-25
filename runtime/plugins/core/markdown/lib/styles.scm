;;; core:markdown — styles.scm. See docs/architecture.md, "Inline styles".

(require "queries.scm")

(provide md-styles/toggler)

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

;;; `span` and each span of `spans` nested with it, one character narrower or wider on each side per level.
(define (md-styles/nested spans span)
  (define (step s delta)
    (let ([node (car s)])
      (findf (lambda (o)
               (and (= (md-preview/capture-start (car o)) (- (md-preview/capture-start node) delta))
                    (= (md-preview/capture-end (car o)) (+ (md-preview/capture-end node) delta))))
             spans)))
  (let up ([s (let down ([s span]) (let ([inner (step s -1)]) (if inner (down inner) s)))] [chain '()])
    (let ([outer (step s 1)])
      (if outer (up outer (cons s chain)) (cons s chain)))))

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
                       [(equal? style "strikethrough") (md-styles/nested spans span)]
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

(define (md-styles/single-cursor? pane)
  (let ([sels (buffer-selections pane)])
    (and (null? (cdr sels))
         (= (selection-anchor (car sels)) (selection-head (car sels))))))

(define (md-styles/wrap! pane marker)
  (let ([back (if (md-styles/single-cursor? pane) "move-left" "undo-selection-change")])
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
            (edit-text! pane (map (lambda (m) (hash 'start-char (car m) 'end-char (cdr m) 'text ""))
                                  (car removal)))
            (when (cdr removal) (call! "move-left" pane)))
          (md-styles/wrap! pane marker)))))
