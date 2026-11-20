;;; core:markdown — destinations.scm. See docs/links.md.

(provide md-destinations/resolve md-destinations/bare-depth md-destinations/bare?
         md-destinations/angle-safe? md-destinations/encode)

(define (md-destinations/resolve pane destination)
  (let ([file (buffer-path pane)])
    (and file (path-join (parent-name file) (car (split-many destination "#"))))))

;;; See docs/links.md's "Destinations".
(define (md-destinations/bare-depth text)
  (and (not (starts-with? text "<"))
       (let loop ([chars (string->list text)] [depth 0])
         (cond [(null? chars) depth]
               [(char-whitespace? (car chars)) #f]
               [(char=? (car chars) #\() (loop (cdr chars) (+ depth 1))]
               [(char=? (car chars) #\)) (and (> depth 0) (loop (cdr chars) (- depth 1)))]
               [else (loop (cdr chars) depth)]))))

(define (md-destinations/bare? text)
  (and (not (equal? text ""))
       (equal? (md-destinations/bare-depth text) 0)))

(define (md-destinations/angle-safe? text)
  (not (or (string-contains? text "<")
           (string-contains? text ">")
           (string-contains? text "\n"))))

(define (md-destinations/encode text)
  (cond [(md-destinations/bare? text) text]
        [(md-destinations/angle-safe? text) (string-append "<" text ">")]
        [else #f]))
