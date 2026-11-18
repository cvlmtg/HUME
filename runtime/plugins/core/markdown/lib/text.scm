;;; core:markdown — text.scm. See README.md.

(require "queries.scm")
(require "render.scm")

(provide md-preview/format-text)

(define (md-preview/text-lines text)
  (let loop ([chars (string->list text)] [at 0] [start 0] [line '()] [lines '()])
    (cond
      [(null? chars) (list->vector (reverse lines))]
      [(char=? (car chars) #\newline)
       (loop (cdr chars) (+ at 1) (+ at 1) '()
             (cons (cons start (list->string (reverse line))) lines))]
      [else (loop (cdr chars) (+ at 1) start (cons (car chars) line) lines)])))

(define (md-preview/text-line-of lines)
  (lambda (char)
    (let loop ([low 0] [high (vector-length lines)])
      (if (< (+ low 1) high)
          (let ([mid (quotient (+ low high) 2)])
            (if (<= (car (vector-ref lines mid)) char)
                (loop mid high)
                (loop low mid)))
          low))))

(define (md-preview/format-text text parsed rule-width)
  (let* ([lines (md-preview/text-lines text)]
         [line-count (vector-length lines)]
         [found (md-preview/elements
                  (parsed-text-captures parsed "markdown" md-preview/block-query)
                  (lambda (ranges)
                    (if (language-has-grammar? "markdown.inline")
                        (parsed-text-captures parsed "markdown.inline" md-preview/inline-query)
                        '()))
                  (list (hash 'start-line 0 'end-line line-count))
                  (md-preview/text-line-of lines)
                  (lambda (line) (vector-ref lines line))
                  line-count
                  rule-width)])
    (hash 'conceals (hash-ref found 'conceals)
          'overlaps 'drop
          'line-backgrounds (hash-ref found 'bands)
          'highlights (hash-ref found 'highlights))))
