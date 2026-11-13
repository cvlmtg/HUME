;;; core:markdown-preview — blocks.scm. See README.md.

(require "queries.scm")

(provide md-preview/block-elements)

(define (md-preview/heading-scope n suffix)
  (string-append "markup.heading." (number->string n) suffix))

(define (md-preview/highlight capture scope)
  (hash 'start-char (md-preview/capture-start capture)
        'end-char (md-preview/capture-end capture)
        'scope scope))

(define (md-preview/band span scope)
  (hash 'line (hash-ref span 'start-line) 'end-line (hash-ref span 'end-line) 'scope scope))

(define (md-preview/heading-texts! groups name n emit!)
  (for-each
    (lambda (c) (emit! 'highlights (md-preview/highlight c (md-preview/heading-scope n ".text"))))
    (md-preview/captured groups name)))

(define (md-preview/atx-headings! groups line-of emit!)
  (for-each
    (lambda (n)
      (for-each
        (lambda (c)
          (let ([start (md-preview/capture-start c)])
            (emit! 'conceals
                   (md-preview/conceal start (md-preview/capture-end c)
                                       (string-append " " (number->string n) " ")
                                       (md-preview/heading-scope n ".marker")
                                       #f))
            (emit! 'bands (hash 'line (line-of start) 'scope (md-preview/heading-scope n ".line")))))
        (md-preview/captured groups (md-preview/heading n)))
      (md-preview/heading-texts! groups (md-preview/heading-text n) n emit!))
    md-preview/atx-levels))

(define (md-preview/setext-headings! groups line-of emit!)
  (for-each
    (lambda (n)
      (for-each
        (lambda (c)
          (emit! 'bands (md-preview/band (md-preview/line-span line-of c)
                                         (md-preview/heading-scope n ".line"))))
        (md-preview/captured groups (md-preview/setext n)))
      (md-preview/heading-texts! groups (md-preview/setext-text n) n emit!))
    md-preview/setext-levels))

(define md-preview/boxes
  (hash "task-unchecked" (cons "☐" "markup.list.unchecked")
        "task-checked" (cons "☑" "markup.list.checked")))

(define (md-preview/lists! groups emit!)
  (let ([tasks (foldl (lambda (t table) (hash-insert table (md-preview/capture-start t) t))
                      (hash)
                      (append (md-preview/captured groups "task-unchecked")
                              (md-preview/captured groups "task-checked")))])
    (for-each
      (lambda (bullet)
        (let* ([start (md-preview/capture-start bullet)]
               [task (hash-try-get tasks (md-preview/capture-end bullet))])
          (emit! 'conceals
                 (if task
                     (let ([glyph (hash-ref md-preview/boxes (car task))])
                       (md-preview/conceal start (md-preview/capture-end task) (car glyph) (cdr glyph) #f))
                     (md-preview/conceal start (+ start 1) "•" #f #f)))))
      (md-preview/captured groups "bullet"))))

(define (md-preview/content-start text)
  (let ([rest (trim-start text)])
    (and (not (string=? rest "")) (- (string-length text) (string-length rest)))))

(define (md-preview/quotes! groups range line-of line-info emit!)
  (let ([marked (box (hash))])
    (define (bar! at)
      (set-box! marked (hash-insert (unbox marked) (line-of at) #t))
      (emit! 'conceals (md-preview/conceal at (+ at 1) "┃" "markup.quote" #f)))
    (for-each (lambda (c) (bar! (md-preview/capture-start c)))
              (md-preview/captured groups "quote-marker"))
    (for-each
      (lambda (c)
        (let ([start (md-preview/capture-start c)] [end (md-preview/capture-end c)])
          (when (< start end)
            (let ([text (md-preview/line-slice start end line-of line-info)])
              (let loop ([chars (string->list text)] [at start])
                (unless (null? chars)
                  (when (char=? (car chars) #\>) (bar! at))
                  (loop (cdr chars) (+ at 1))))))))
      (append (md-preview/captured groups "continuation")
              (md-preview/captured groups "fence-continuation")))
    (for-each
      (lambda (block)
        (let* ([span (md-preview/line-span line-of block)]
               [end (min (hash-ref range 'end-line) (hash-ref span 'end-line))])
          (let loop ([line (max (hash-ref range 'start-line) (hash-ref span 'start-line))])
            (when (< line end)
              (unless (hash-contains? (unbox marked) line)
                (set-box! marked (hash-insert (unbox marked) line #t))
                (let* ([info (line-info line)]
                       [content (md-preview/content-start (cdr info))])
                  (when content
                    (let ([at (+ (car info) content)])
                      (emit! 'conceals (md-preview/conceal at at "┃ " "markup.quote" #f))))))
              (loop (+ line 1))))))
      (md-preview/captured groups "quote"))))

(define (md-preview/rules! groups rule-width emit!)
  (let ([rule (make-string rule-width #\─)])
    (for-each
      (lambda (c)
        (emit! 'conceals (md-preview/conceal (md-preview/capture-start c) (md-preview/capture-end c)
                                             rule "punctuation.special" #f)))
      (md-preview/captured groups "rule"))))

(define (md-preview/fences! groups line-of emit!)
  (let ([delimiters (md-preview/captured groups "fence-delimiter")])
    (for-each
      (lambda (fence)
        (let ([span (md-preview/line-span line-of fence)])
          (emit! 'bands (md-preview/band span "markup.raw.block.line"))
          (for-each
            (lambda (d)
              (when (md-preview/inside? d fence)
                (emit! 'conceals (md-preview/conceal (md-preview/capture-start d) (md-preview/capture-end d)
                                                     #f #f span))))
            delimiters)))
      (md-preview/captured groups "fence"))))

(define (md-preview/block-elements groups range line-of line-info rule-width)
  (let* ([found (box (hash 'conceals '() 'bands '() 'highlights '()))]
         [emit! (lambda (kind item)
                  (let ([table (unbox found)])
                    (set-box! found (hash-insert table kind (cons item (hash-ref table kind))))))])
    (md-preview/atx-headings! groups line-of emit!)
    (md-preview/setext-headings! groups line-of emit!)
    (md-preview/lists! groups emit!)
    (md-preview/quotes! groups range line-of line-info emit!)
    (md-preview/rules! groups rule-width emit!)
    (md-preview/fences! groups line-of emit!)
    (unbox found)))
