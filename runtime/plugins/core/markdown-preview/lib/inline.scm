;;; core:markdown-preview — inline.scm. See README.md.

(require "queries.scm")
(require "entities.scm")

(provide md-preview/inline-elements)

(define (md-preview/hidden start end) (md-preview/conceal start end #f #f #f))

(define (md-preview/still-open stack at)
  (if (and (pair? stack) (<= (md-preview/capture-end (car stack)) at))
      (md-preview/still-open (cdr stack) at)
      stack))

(define (md-preview/innermost stack fits?)
  (cond
    [(null? stack) (error "markdown-preview: an inline capture outside its construct")]
    [(fits? (car stack)) (car stack)]
    [else (md-preview/innermost (cdr stack) fits?)]))

(define (md-preview/span-delimiter stack c)
  (let ([span (md-preview/innermost stack (lambda (o) (equal? (car o) "span")))])
    (list (md-preview/conceal (md-preview/capture-start c) (md-preview/capture-end c)
                              #f #f (list-ref span 3)))))

(define (md-preview/link-text stack c)
  (let* ([start (md-preview/capture-start c)]
         [link (md-preview/innermost stack
                                     (lambda (o) (and (equal? (car o) "link")
                                                      (< (md-preview/capture-start o) start))))])
    (list (md-preview/hidden (md-preview/capture-start link) start)
          (md-preview/hidden (md-preview/capture-end c) (md-preview/capture-end link)))))

(define (md-preview/autolink c)
  (let ([start (md-preview/capture-start c)] [end (md-preview/capture-end c)])
    (list (md-preview/hidden start (+ start 1)) (md-preview/hidden (- end 1) end))))

(define (md-preview/escape c)
  (let ([start (md-preview/capture-start c)])
    (list (md-preview/hidden start (+ start 1)))))

(define (md-preview/entity c line-of line-info)
  (let* ([start (md-preview/capture-start c)]
         [end (md-preview/capture-end c)]
         [text (md-preview/entity-text (md-preview/line-slice start end line-of line-info))])
    (if text (list (md-preview/conceal start end text #f #f)) '())))

(define (md-preview/inline-elements captures line-of line-info)
  (let loop ([cs captures] [stack '()] [conceals '()])
    (if (null? cs)
        conceals
        (let* ([c (car cs)]
               [name (car c)]
               [stack (md-preview/still-open stack (md-preview/capture-start c))]
               [next (lambda (stack found) (loop (cdr cs) stack (append found conceals)))])
          (cond
            [(equal? name "span")
             (let ([reveal (md-preview/line-span line-of c)])
               (next (cons (append c (list reveal)) stack) '()))]
            [(equal? name "link") (next (cons c stack) '())]
            [(equal? name "span-delimiter") (next stack (md-preview/span-delimiter stack c))]
            [(equal? name "link-text") (next stack (md-preview/link-text stack c))]
            [(equal? name "autolink") (next stack (md-preview/autolink c))]
            [(equal? name "escape") (next stack (md-preview/escape c))]
            [(equal? name "entity") (next stack (md-preview/entity c line-of line-info))]
            [else (error (string-append "markdown-preview: unknown inline capture " name))])))))
