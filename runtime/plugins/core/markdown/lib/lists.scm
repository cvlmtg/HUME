;;; core:markdown — lists.scm. See docs/lists.md.

(require "queries.scm")

(provide md-lists/item md-lists/past-prefix? md-lists/empty? md-lists/ended-edit
         md-lists/events md-lists/removed-events md-lists/renumber md-lists/prefixes)

(define (md-lists/on-line captures start end)
  (filter (lambda (c) (and (>= (md-preview/capture-start c) start)
                           (< (md-preview/capture-start c) end)))
          captures))

(define (md-lists/blank? c) (or (char=? c #\space) (char=? c #\tab)))

(define (md-lists/digit? c) (and (char>=? c #\0) (char<=? c #\9)))

(define (md-lists/skip-blanks text col)
  (if (and (< col (string-length text)) (md-lists/blank? (string-ref text col)))
      (md-lists/skip-blanks text (+ col 1))
      col))

(define (md-lists/last-index-of text ch end)
  (let loop ([i (- end 1)])
    (cond
      [(< i 0) #f]
      [(char=? (string-ref text i) ch) i]
      [else (loop (- i 1))])))

(define (md-lists/token-length text col)
  (if (and (< col (string-length text)) (not (md-lists/blank? (string-ref text col))))
      (+ 1 (md-lists/token-length text (+ col 1)))
      0))

(define (md-lists/col capture start)
  (- (md-preview/capture-start capture) start))

(define (md-lists/end-col capture start text)
  (min (string-length text) (- (md-preview/capture-end capture) start)))

(define (md-lists/quote-prefix? capture start text)
  (string-contains? (substring text (md-lists/col capture start) (md-lists/end-col capture start text))
                    ">"))

(define (md-lists/quotes groups start end text from)
  (filter (lambda (c) (and (>= (md-lists/col c start) from) (md-lists/quote-prefix? c start text)))
          (md-lists/on-line (md-preview/captured groups "quote-prefix") start end)))

(define (md-lists/past-quotes quotes start text)
  (md-lists/skip-blanks text (apply max (map (lambda (c) (md-lists/end-col c start text)) quotes))))

;;; See docs/lists.md's "Prefixes".
(define (md-lists/item line groups line-info)
  (let* ([info (line-info line)]
         [start (car info)]
         [text (cdr info)]
         [end (+ start (string-length text) 1)]
         [markers (sort (md-lists/on-line (append (md-preview/captured groups "bullet")
                                                  (md-preview/captured groups "ordered"))
                                          start end)
                        (lambda (a b) (< (md-preview/capture-start a) (md-preview/capture-start b))))]
         [item (hash 'start start 'text text 'markers markers 'marker #f 'task #f)])
    (if (pair? markers)
        (let* ([marker (list-ref markers (- (length markers) 1))]
               [after (md-lists/end-col marker start text)]
               [task (findf (lambda (c) (= (md-lists/col c start) after))
                            (md-lists/on-line (md-preview/captured groups "task") start end))]
               [after (if task (md-lists/end-col task start text) after)]
               [quotes (md-lists/quotes groups start end text after)])
          (if (pair? quotes)
              (hash-insert item 'prefix-end (md-lists/past-quotes quotes start text))
              (hash-insert (hash-insert (hash-insert item 'marker marker) 'task task)
                           'prefix-end (md-lists/skip-blanks text after))))
        (let ([quotes (md-lists/quotes groups start end text 0)])
          (and (pair? quotes)
               (hash-insert item 'prefix-end (md-lists/past-quotes quotes start text)))))))

(define (md-lists/past-prefix? item head)
  (>= (- head (hash-ref item 'start)) (hash-ref item 'prefix-end)))

(define (md-lists/empty? item)
  (= (hash-ref item 'prefix-end) (string-length (hash-ref item 'text))))

;;; See docs/lists.md's "Prefixes".
(define (md-lists/next-prefix item number)
  (let* ([text (hash-ref item 'text)]
         [start (hash-ref item 'start)]
         [marker (hash-ref item 'marker)]
         [task (hash-ref item 'task)]
         [outer (filter (lambda (m) (not (equal? m marker))) (hash-ref item 'markers))]
         [end (hash-ref item 'prefix-end)])
    (let loop ([col 0] [parts '()])
      (cond
        [(>= col end) (apply string-append (reverse parts))]
        [(and marker (= col (md-lists/col marker start)))
         (let* ([token (md-lists/token-length text col)]
                [next (if number
                          (string-append (number->string number)
                                         (substring text (+ col token -1) (+ col token)))
                          (substring text col (+ col token)))])
           (loop (+ col token) (cons next parts)))]
        [(and task (= col (md-lists/col task start)))
         (loop (md-lists/end-col task start text) (cons "[ ]" parts))]
        [(findf (lambda (m) (and (>= col (md-lists/col m start))
                                 (< col (md-lists/end-col m start text))))
                outer)
         (loop (+ col 1) (cons " " parts))]
        [else (loop (+ col 1) (cons (substring text col (+ col 1)) parts))]))))

;;; See docs/lists.md's "Ending a list".
(define (md-lists/kept-col item)
  (let ([text (hash-ref item 'text)]
        [markers (hash-ref item 'markers)])
    (if (pair? markers)
        (let* ([cut (md-lists/col (car markers) (hash-ref item 'start))]
               [bar (md-lists/last-index-of text #\> cut)])
          (cond
            [(not bar) 0]
            [(and (< (+ bar 1) cut) (md-lists/blank? (string-ref text (+ bar 1))))
             (+ bar 2)]
            [else (+ bar 1)]))
        (or (md-lists/last-index-of text #\> (hash-ref item 'prefix-end)) 0))))

(define (md-lists/ended-edit item)
  (let ([start (hash-ref item 'start)])
    (hash 'start-char (+ start (md-lists/kept-col item))
          'end-char (+ start (hash-ref item 'prefix-end))
          'text "")))

;; ── Renumbering ───────────────────────────────────────────────────────────────

(define (md-lists/width capture)
  (- (md-preview/capture-end capture) (md-preview/capture-start capture)))

(define (md-lists/list-of lists marker)
  (foldl (lambda (l best)
           (if (and (md-preview/inside? marker l)
                    (or (not best) (< (md-lists/width l) (md-lists/width best))))
               l
               best))
         #f
         lists))

(define (md-lists/digits marker line-of line-info)
  (let* ([start (md-preview/capture-start marker)]
         [info (line-info (line-of start))]
         [text (cdr info)]
         [col (- start (car info))])
    (let loop ([end col])
      (if (and (< end (string-length text)) (md-lists/digit? (string-ref text end)))
          (loop (+ end 1))
          (substring text col end)))))

(define (md-lists/number marker line-of line-info)
  (string->number (md-lists/digits marker line-of line-info)))

;;; See docs/lists.md's "Renumbering".
(define (md-lists/renumber groups events line-of line-info)
  (let* ([lists (md-preview/captured groups "list")]
         [ordered (sort (md-preview/captured groups "ordered")
                        (lambda (a b) (< (md-preview/capture-start a) (md-preview/capture-start b))))]
         [indexed (let loop ([es events] [i 0] [out '()])
                    (if (null? es)
                        (reverse out)
                        (loop (cdr es) (+ i 1) (cons (cons i (car es)) out))))]
         [marker-of (lambda (e) (car (cdr e)))]
         [numbered (filter (lambda (e) (let ([m (marker-of e)]) (and m (equal? (car m) "ordered"))))
                           indexed)]
         [kind-of (lambda (e) (cdr (cdr e)))]
         [touched (foldl (lambda (e acc)
                           (let ([l (md-lists/list-of lists (marker-of e))])
                             (if (member l acc) acc (cons l acc))))
                         '()
                         numbered)]
         [numbers (box (hash))]
         [edits (box '())])
    (define (number m) (md-lists/number m line-of line-info))
    (define (assign! es counter)
      (foldl (lambda (e n)
               (set-box! numbers (hash-insert (unbox numbers) (car e) n))
               (+ n 1))
             counter
             es))
    (define (renumber! m counter)
      (let ([digits (md-lists/digits m line-of line-info)])
        (unless (= counter (string->number digits))
          (let ([s (md-preview/capture-start m)])
            (set-box! edits
                      (cons (hash 'start-char s
                                  'end-char (+ s (string-length digits))
                                  'text (number->string counter))
                            (unbox edits)))))))
    (for-each
      (lambda (l)
        (let walk ([items (filter (lambda (m) (equal? (md-lists/list-of lists m) l)) ordered)]
                   [counter #f])
          (unless (null? items)
            (let* ([m (car items)]
                   [at (filter (lambda (e) (equal? (marker-of e) m)) numbered)]
                   [of-kind (lambda (k) (filter (lambda (e) (equal? (kind-of e) k)) at))]
                   [counter (if (pair? (of-kind 'before))
                                (assign! (of-kind 'before) (or counter (number m)))
                                counter)]
                   [counter (cond
                              [(pair? (of-kind 'remove)) (or counter (number m))]
                              [counter (renumber! m counter) (+ counter 1)]
                              [else #f])]
                   [counter (if (pair? (of-kind 'after))
                                (assign! (of-kind 'after) (or counter (+ (number m) 1)))
                                counter)])
              (walk (cdr items) counter)))))
      touched)
    (cons (unbox edits)
          (map (lambda (e) (hash-try-get (unbox numbers) (car e))) indexed))))

;;; One event of `kind` per item, on its marker, the `#f`s of lines with no prefix left out.
(define (md-lists/events items kind)
  (map (lambda (item) (cons (hash-ref item 'marker) kind)) (filter (lambda (item) item) items)))

;;; One `'remove` event per marker `md-lists/ended-edit` deletes.
(define (md-lists/removed-events items)
  (apply append
         (map (lambda (item)
                (let ([kept (md-lists/kept-col item)] [start (hash-ref item 'start)])
                  (map (lambda (m) (cons m 'remove))
                       (filter (lambda (m) (>= (md-lists/col m start) kept))
                               (hash-ref item 'markers)))))
              (filter (lambda (item) item) items))))

;;; Per item, its next line's prefix, or `#f` for a line with no prefix.
(define (md-lists/prefixes items numbers)
  (let loop ([items items] [numbers numbers] [prefixes '()])
    (cond
      [(null? items) (reverse prefixes)]
      [(car items)
       (loop (cdr items) (cdr numbers)
             (cons (md-lists/next-prefix (car items) (car numbers)) prefixes))]
      [else (loop (cdr items) numbers (cons #f prefixes))])))
