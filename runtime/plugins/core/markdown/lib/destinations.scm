;;; core:markdown — destinations.scm. See docs/links.md.

(provide md-destinations/split md-destinations/resolve md-destinations/bare-depth
         md-destinations/bare? md-destinations/angle-safe? md-destinations/encode
         md-destinations/decode)

;;; See docs/links.md's "Destinations".
(define (md-destinations/split destination)
  (let ([parts (split-once destination "#")])
    (if (pair? parts) (cons (car parts) (cadr parts)) (cons destination #f))))

(define (md-destinations/resolve pane path)
  (let ([file (buffer-path pane)])
    (and file (path-join (parent-name file) path))))

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

(define (md-destinations/hex-digit byte)
  (cond [(<= 48 byte 57) (- byte 48)]
        [(<= 65 byte 70) (- byte 55)]
        [(<= 97 byte 102) (- byte 87)]
        [else #f]))

;;; See docs/links.md's "Following links".
(define (md-destinations/decode text)
  (if (string-contains? text "%")
      (let loop ([bytes (bytes->list (string->bytes text))] [out '()])
        (cond
          [(null? bytes)
           (with-handler
             (lambda (_) (error (string-append "markdown: " text " is not valid percent-encoded UTF-8")))
             (bytes->string/utf8 (list->bytes (reverse out))))]
          [(and (= (car bytes) 37) (pair? (cdr bytes)) (pair? (cddr bytes))
                (let ([high (md-destinations/hex-digit (cadr bytes))]
                      [low (md-destinations/hex-digit (caddr bytes))])
                  (and high low (+ (* 16 high) low))))
           => (lambda (byte) (loop (cdddr bytes) (cons byte out)))]
          [else (loop (cdr bytes) (cons (car bytes) out))]))
      text))
