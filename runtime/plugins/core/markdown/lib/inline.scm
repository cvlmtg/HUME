;;; core:markdown — inline.scm. See docs/elements.md.

(require "queries.scm")
(require "entities.scm")

(provide md-preview/inline-elements)

(define (md-preview/hidden start end) (md-preview/conceal start end #f #f #f))

(define (md-preview/link-text link text)
  (list (md-preview/hidden (md-preview/capture-start link) (md-preview/capture-start text))
        (md-preview/hidden (md-preview/capture-end text) (md-preview/capture-end link))))

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

(define (md-preview/inline-elements matches line-of line-info)
  (foldl (lambda (run out)
           (let* ([c (car run)] [name (car c)])
             (append
               (cond
                 [(equal? name "span")
                  (md-preview/hidden-revealing (md-preview/children run) (md-preview/line-span line-of c))]
                 [(equal? name "link") (md-preview/link-text c (car (md-preview/children run)))]
                 [(equal? name "autolink") (md-preview/autolink c)]
                 [(equal? name "escape") (md-preview/escape c)]
                 [(equal? name "entity") (md-preview/entity c line-of line-info)]
                 [else (error (string-append "markdown: unknown inline capture " name))])
               out)))
         '()
         (md-preview/runs matches)))
