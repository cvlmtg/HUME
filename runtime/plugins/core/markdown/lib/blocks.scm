;;; core:markdown — blocks.scm. See docs/elements.md.

(require "queries.scm")
(require "state.scm")

(provide md-preview/block-elements)

(define (md-preview/heading-scope n suffix)
  (string-append "markup.heading." (number->string n) suffix))

(define md-preview/level-glyphs '("⠈" "⠘" "⠸" "⠹" "⠻" "⠿"))

(define (md-preview/heading-marker n)
  (string-append (list-ref md-preview/level-glyphs (- n 1)) "▌"))

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
                                       (md-preview/heading-marker n)
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
  (hash md-preview/unchecked-box (cons "☐" "markup.list.unchecked")
        md-preview/checked-box (cons "☑" "markup.list.checked")))

(define (md-preview/lists! groups line-of line-info emit!)
  (for-each
    (lambda (run)
      (let* ([marker (car run)]
             [start (md-preview/capture-start marker)]
             [info (line-info (line-of start))]
             [task (md-preview/task-box marker (md-preview/child run) (car info) (cdr info))])
        (emit! 'conceals
               (if task
                   (let ([glyph (hash-ref md-preview/boxes (car task))])
                     (md-preview/conceal start (md-preview/capture-end task) (car glyph) (cdr glyph) #f))
                   (md-preview/conceal start (+ start 1) "•" #f #f)))))
    (md-preview/runs-of groups "bullet")))

(define (md-preview/quotes! groups ranges line-of line-info emit!)
  (let ([marked (box (hash))])
    (define (bar! at)
      (md-preview/box-insert! marked (line-of at) #t)
      (emit! 'conceals (md-preview/conceal at (+ at 1) "┃" "markup.quote" #f)))
    (define (lazy-bar! line)
      (unless (hash-contains? (unbox marked) line)
        (md-preview/box-insert! marked line #t)
        (let* ([info (line-info line)]
               [col (md-preview/skip-blanks (cdr info) 0)])
          (when (< col (string-length (cdr info)))
            (let ([at (+ (car info) col)])
              (emit! 'conceals (md-preview/conceal at at "┃ " "markup.quote" #f)))))))
    (for-each
      (lambda (c)
        (let ([info (line-info (line-of (md-preview/capture-start c)))])
          (for-each (lambda (col) (bar! (+ (car info) col)))
                    (md-preview/quote-cols c (car info) (cdr info) '()))))
      (append (md-preview/captured groups "quote-marker")
              (md-preview/captured groups "continuation")
              (md-preview/captured groups "fence-continuation")))
    (for-each
      (lambda (block)
        (let ([span (md-preview/line-span line-of block)])
          (for-each
            (lambda (range)
              (let ([end (min (hash-ref range 'end-line) (hash-ref span 'end-line))])
                (let loop ([line (max (hash-ref range 'start-line) (hash-ref span 'start-line))])
                  (when (< line end)
                    (lazy-bar! line)
                    (loop (+ line 1))))))
            ranges)))
      (md-preview/captured groups "quote"))))

(define (md-preview/rules! groups rule-width emit!)
  (let ([rule (make-string rule-width #\─)])
    (for-each
      (lambda (c)
        (emit! 'conceals (md-preview/conceal (md-preview/capture-start c) (md-preview/capture-end c)
                                             rule "punctuation.special" #f)))
      (md-preview/captured groups "rule"))))

(define (md-preview/fences! groups line-of emit!)
  (for-each
    (lambda (run)
      (let ([span (md-preview/line-span line-of (car run))])
        (emit! 'bands (md-preview/band span "markup.raw.block.line"))
        (for-each (lambda (c) (emit! 'conceals c))
                  (md-preview/hidden-revealing (md-preview/children run) span))))
    (md-preview/runs-of groups "fence")))

(define (md-preview/block-elements groups ranges line-of line-info rule-width)
  (let* ([found (box (hash 'conceals '() 'bands '() 'highlights '()))]
         [emit! (lambda (kind item)
                  (md-preview/box-insert! found kind (cons item (hash-ref (unbox found) kind))))])
    (md-preview/atx-headings! groups line-of emit!)
    (md-preview/setext-headings! groups line-of emit!)
    (md-preview/lists! groups line-of line-info emit!)
    (md-preview/quotes! groups ranges line-of line-info emit!)
    (md-preview/rules! groups rule-width emit!)
    (md-preview/fences! groups line-of emit!)
    (unbox found)))
