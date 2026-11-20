;;; core:markdown — lists.scm. See docs/lists.md.

(require "queries.scm")
(require "state.scm")

(provide md-lists/distinct md-lists/items md-lists/past-prefix? md-lists/empty? md-lists/ended-edit
         md-lists/events md-lists/removed-events md-lists/renumber md-lists/prefixes
         md-lists/edited-markers md-lists/task-edits)

(define (md-lists/distinct lines)
  (reverse (foldl (lambda (line kept) (if (and (pair? kept) (= (car kept) line)) kept (cons line kept)))
                  '()
                  lines)))

(define (md-lists/token-length text col)
  (- (md-preview/skip-while text col (lambda (c) (not (md-preview/blank? c)))) col))

(define (md-lists/col capture start)
  (- (md-preview/capture-start capture) start))

(define (md-lists/past-bars bars text)
  (md-preview/skip-blanks text (+ (apply max bars) 1)))

;;; See docs/lists.md's "Prefixes".
(define (md-lists/item info marked quote-prefixes)
  (let* ([start (car info)]
         [text (cdr info)]
         [markers (map car marked)]
         [bars (foldl (lambda (c tail) (md-preview/quote-cols c start text tail))
                      '()
                      (reverse quote-prefixes))]
         [item (hash 'start start 'text text 'markers markers 'bars bars 'marker #f 'task #f)])
    (if (pair? marked)
        (let* ([last-marked (last marked)]
               [marker (car last-marked)]
               [task (md-preview/task-box marker (cdr last-marked) start text)]
               [after (md-preview/end-col (or task marker) start text)]
               [inner (filter (lambda (col) (>= col after)) bars)])
          (if (pair? inner)
              (hash-insert item 'prefix-end (md-lists/past-bars inner text))
              (hash-insert (hash-insert (hash-insert item 'marker marker) 'task task)
                           'prefix-end (md-preview/skip-blanks text after))))
        (and (pair? bars)
             (hash-insert item 'prefix-end (md-lists/past-bars bars text))))))

;;; Per line of `lines`, ascending with repeats allowed, its item or `#f`.
(define (md-lists/items lines groups line-info)
  (let* ([unique (md-lists/distinct lines)]
         [infos (map line-info unique)]
         [spans (map (lambda (info) (cons (car info) (+ (car info) (string-length (cdr info))))) infos)]
         [marked (sort (map (lambda (run) (cons (car run) (md-preview/child run)))
                            (append (md-preview/runs-of groups "bullet") (md-preview/runs-of groups "ordered")))
                       (lambda (a b) (md-preview/capture-before? (car a) (car b))))]
         [items (map md-lists/item
                     infos
                     (md-preview/bucket spans marked (lambda (m) (md-preview/capture-start (car m))))
                     (md-preview/bucket spans (md-preview/captured groups "quote-prefix")
                                        md-preview/capture-start))])
    (let loop ([lines lines] [unique unique] [items items] [out '()])
      (cond
        [(null? lines) (reverse out)]
        [(= (car lines) (car unique)) (loop (cdr lines) unique items (cons (car items) out))]
        [else (loop lines (cdr unique) (cdr items) out)]))))

(define (md-lists/past-prefix? item head)
  (>= (- head (hash-ref item 'start)) (hash-ref item 'prefix-end)))

(define (md-lists/empty? item)
  (= (hash-ref item 'prefix-end) (string-length (hash-ref item 'text))))

;;; See docs/lists.md's "Prefixes".
(define (md-lists/next-prefix item number)
  (let* ([text (hash-ref item 'text)]
         [start (hash-ref item 'start)]
         [marker (hash-ref item 'marker)]
         [task (hash-ref item 'task)])
    (define (part capture)
      (let ([from (md-lists/col capture start)])
        (cond
          [(equal? capture task) (cons (md-preview/end-col task start text) "[ ]")]
          [(equal? capture marker)
           (let ([to (+ from (md-lists/token-length text from))])
             (cons to (if number
                          (string-append (number->string number) (substring text (- to 1) to))
                          (substring text from to))))]
          [else
           (let ([to (md-preview/end-col capture start text)])
             (cons to (make-string (- to from) #\space)))])))
    (let loop ([col 0]
               [captures (if task (append (hash-ref item 'markers) (list task)) (hash-ref item 'markers))]
               [parts '()])
      (if (null? captures)
          (apply string-append (reverse (cons (substring text col (hash-ref item 'prefix-end)) parts)))
          (let ([next (part (car captures))])
            (loop (car next)
                  (cdr captures)
                  (cons (cdr next)
                        (cons (substring text col (md-lists/col (car captures) start)) parts))))))))

;;; See docs/lists.md's "Ending a list".
(define (md-lists/kept-col item)
  (let ([text (hash-ref item 'text)]
        [markers (hash-ref item 'markers)]
        [bars (hash-ref item 'bars)])
    (if (pair? markers)
        (let* ([cut (md-lists/col (car markers) (hash-ref item 'start))]
               [before (filter (lambda (col) (< col cut)) bars)])
          (if (null? before)
              0
              (let ([bar (apply max before)])
                (if (and (< (+ bar 1) cut) (md-preview/blank? (string-ref text (+ bar 1))))
                    (+ bar 2)
                    (+ bar 1)))))
        (apply max bars))))

(define (md-lists/ended-edit item)
  (let ([start (hash-ref item 'start)])
    (hash 'start-char (+ start (md-lists/kept-col item))
          'end-char (+ start (hash-ref item 'prefix-end))
          'text "")))

;; ── Renumbering ───────────────────────────────────────────────────────────────

(define (md-lists/digits marker line-of line-info)
  (let* ([start (md-preview/capture-start marker)]
         [info (line-info (line-of start))]
         [text (cdr info)]
         [col (- start (car info))])
    (substring text col (md-preview/skip-while text col char-digit?))))

;;; See docs/lists.md's "Renumbering".
(define (md-lists/renumber groups events line-of line-info)
  (let* ([indexed (map cons (range 0 (length events)) events)]
         [marker-of (lambda (e) (car (cdr e)))]
         [numbered (filter (lambda (e) (let ([m (marker-of e)]) (and m (equal? (car m) "ordered"))))
                           indexed)]
         [kind-of (lambda (e) (cdr (cdr e)))]
         [at-marker (md-preview/group-by (lambda (e) (md-preview/capture-start (marker-of e))) numbered)]
         [numbers (box (hash))]
         [edits (box '())])
    (define (number m) (string->number (md-lists/digits m line-of line-info)))
    (define (assign! es counter)
      (foldl (lambda (e n)
               (md-preview/box-insert! numbers (car e) n)
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
    (unless (null? numbered)
      (for-each
        (lambda (run)
          (let walk ([items (md-preview/children run)] [counter #f])
            (unless (null? items)
              (let* ([m (car items)]
                     [at (or (hash-try-get at-marker (md-preview/capture-start m)) '())]
                     [of-kind (lambda (k) (filter (lambda (e) (equal? (kind-of e) k)) at))]
                     [befores (of-kind 'before)]
                     [removes (of-kind 'remove)]
                     [anchors (of-kind 'anchor)]
                     [afters (of-kind 'after)]
                     [counter (if (pair? befores) (assign! befores (or counter (number m))) counter)]
                     [counter (cond
                                [(pair? anchors) (+ (number m) 1)]
                                [(pair? removes) (or counter (number m))]
                                [counter (renumber! m counter) (+ counter 1)]
                                [else #f])]
                     [counter (if (pair? afters) (assign! afters (or counter (+ (number m) 1))) counter)])
                (walk (cdr items) counter)))))
        (md-preview/runs-of groups "list")))
    (cons (unbox edits)
          (map (lambda (e) (hash-try-get (unbox numbers) (car e))) indexed))))

;;; One event of `kind` per item, on its marker, or on `#f` for a line with no prefix.
(define (md-lists/events items kind)
  (map (lambda (item) (cons (and item (hash-ref item 'marker)) kind)) items))

;;; One `'remove` event per marker `md-lists/ended-edit` deletes from `items`, none of them `#f`.
(define (md-lists/removed-events items)
  (call! "stdlib/concat"
    (map (lambda (item)
           (let ([kept (md-lists/kept-col item)] [start (hash-ref item 'start)])
             (map (lambda (m) (cons m 'remove))
                  (filter (lambda (m) (>= (md-lists/col m start) kept))
                          (hash-ref item 'markers)))))
         items)))

;;; Whether `hunks` changed the ordered marker `m`'s number. See docs/lists.md's "Renumbering after an edit".
(define (md-lists/number-changed? hunks line-of line-info m)
  (let* ([start (md-preview/capture-start m)]
         [line (line-of start)]
         [hunk (findf (lambda (h) (and (<= (hash-ref h 'new-start-line) line)
                                       (< line (hash-ref h 'new-end-line))))
                      hunks)])
    (and hunk
         (let ([words (hash-ref hunk 'words)])
           (or (not words)
               (let* ([info (line-info line)]
                      [line-end (+ (car info) (string-length (cdr info)))]
                      [digits-end (+ start (string-length (md-lists/digits m line-of line-info)))]
                      [on-line (filter (lambda (span) (let ([s (hash-ref span 'start-char)])
                                                        (and (<= (car info) s) (<= s line-end))))
                                       (hash-ref words 'new))])
                 (or (null? on-line)
                     (and (findf (lambda (span) (and (< (hash-ref span 'start-char) digits-end)
                                                     (> (hash-ref span 'end-char) start)))
                                 on-line)
                          #t))))))))

;;; An `'anchor` event per ordered marker in `groups` whose number `hunks` changed.
(define (md-lists/edited-markers groups hunks line-of line-info)
  (map (lambda (m) (cons m 'anchor))
       (filter (lambda (m) (md-lists/number-changed? hunks line-of line-info m))
               (call! "stdlib/concat" (map md-preview/children (md-preview/runs-of groups "list"))))))

;;; Per item, its next line's prefix, or `#f` for a line with no prefix.
(define (md-lists/prefixes items numbers)
  (map (lambda (item number) (and item (md-lists/next-prefix item number))) items numbers))

;; ── Task boxes ────────────────────────────────────────────────────────────────

(define (md-lists/task? item)
  (and item (hash-ref item 'task)))

(define (md-lists/checked? item)
  (equal? (car (hash-ref item 'task)) md-preview/checked-box))

;;; See docs/lists.md's "Task boxes".
(define (md-lists/task-edits items primary-items)
  (let ([decider (or (findf md-lists/task? primary-items) (findf md-lists/task? items))])
    (if decider
        (let ([check? (not (md-lists/checked? decider))])
          (map (lambda (item)
                 (let ([mark (+ (md-preview/capture-start (hash-ref item 'task)) 1)])
                   (hash 'start-char mark 'end-char (+ mark 1) 'text (if check? "x" " "))))
               (filter (lambda (item) (and (md-lists/task? item) (not (eq? (md-lists/checked? item) check?))))
                       items)))
        '())))
