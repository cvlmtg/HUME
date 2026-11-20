;;; core:markdown — lists.scm. See docs/lists.md.

(require "queries.scm")
(require "state.scm")

(provide md-lists/distinct md-lists/items md-lists/past-prefix? md-lists/empty? md-lists/ended-edit
         md-lists/events md-lists/removed-events md-lists/renumber md-lists/prefixes
         md-lists/edited-hunks md-lists/edited-spans md-lists/edited-events md-lists/task-edits)

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
         [kind-of (lambda (e) (let ([kind (cdr (cdr e))]) (if (pair? kind) (car kind) kind)))]
         [start-of (lambda (e) (cdr (cdr (cdr e))))]
         [at-marker (md-preview/group-by (lambda (e) (md-preview/capture-start (marker-of e))) numbered)]
         [numbers (box (hash))]
         [edits (box '())])
    (define (number m) (string->number (md-lists/digits m line-of line-info)))
    (define (events-at m) (or (hash-try-get at-marker (md-preview/capture-start m)) '()))
    (define (of-kind at k) (filter (lambda (e) (equal? (kind-of e) k)) at))
    (define (uniform items)
      (let ([kept (map number (filter (lambda (m) (null? (of-kind (events-at m) 'anchor))) items))])
        (and (>= (length kept) 2)
             (null? (filter (lambda (n) (not (= n (car kept)))) kept))
             (car kept))))
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
          (let* ([items (md-preview/children run)]
                 [same (uniform items)])
            (if same
                (for-each (lambda (m)
                            (let ([at (events-at m)])
                              (for-each (lambda (e) (md-preview/box-insert! numbers (car e) same))
                                        (append (of-kind at 'before) (of-kind at 'after)))))
                          items)
                (let walk ([items items] [counter #f])
                  (unless (null? items)
                    (let* ([m (car items)]
                           [at (events-at m)]
                           [befores (of-kind at 'before)]
                           [starts (of-kind at 'start)]
                           [afters (of-kind at 'after)]
                           [counter (if (pair? befores) (assign! befores (or counter (number m))) counter)]
                           [counter (cond
                                      [(pair? (of-kind at 'anchor)) (+ (number m) 1)]
                                      [(pair? starts)
                                       (let ([n (start-of (car starts))]) (renumber! m n) (+ n 1))]
                                      [(pair? (of-kind at 'remove)) (or counter (number m))]
                                      [counter (renumber! m counter) (+ counter 1)]
                                      [(pair? (of-kind at 'resume)) (+ (number m) 1)]
                                      [else #f])]
                           [counter (if (pair? afters) (assign! afters (or counter (+ (number m) 1))) counter)])
                      (walk (cdr items) counter)))))))
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

;;; The ordered markers in `text`'s container prefix, as `(col . number)`.
(define (md-lists/line-markers text)
  (let ([len (string-length text)])
    (define (ends? col) (or (= col len) (md-preview/blank? (string-ref text col))))
    (let loop ([col 0] [out '()])
      (let ([col (md-preview/skip-while text col (lambda (c) (or (md-preview/blank? c) (char=? c #\>))))])
        (if (= col len)
            (reverse out)
            (let* ([c (string-ref text col)]
                   [end (md-preview/skip-while text col char-digit?)])
              (cond
                [(or (char=? c #\-) (char=? c #\*) (char=? c #\+))
                 (if (ends? (+ col 1)) (loop (+ col 1) out) (reverse out))]
                [(and (< col end) (<= (- end col) 9) (< end len)
                      (or (char=? (string-ref text end) #\.) (char=? (string-ref text end) #\)))
                      (ends? (+ end 1)))
                 (loop (+ end 1) (cons (cons col (string->number (substring text col end))) out))]
                [else (reverse out)])))))))

(define (md-lists/numbers-at markers col)
  (map cdr (filter (lambda (m) (= (car m) col)) markers)))

;;; Per hunk of `hunks` that changed the numbers its lines' ordered markers hold, the hunk, its old
;;; lines' markers, and the columns whose numbers changed, as `(hunk old . cols)`.
(define (md-lists/edited-hunks hunks)
  (filter (lambda (e) (pair? (cdr (cdr e))))
          (map (lambda (hunk)
                 (let* ([side (lambda (key) (call! "stdlib/concat" (map md-lists/line-markers (hash-ref hunk key))))]
                        [old (side 'old-lines)]
                        [new (side 'new-lines)])
                   (cons hunk
                         (cons old
                               (filter (lambda (col) (not (equal? (md-lists/numbers-at old col)
                                                                  (md-lists/numbers-at new col))))
                                       (md-lists/distinct (sort (map car (append old new)) <)))))))
               hunks)))

;;; The line ranges the lists `edited` renumbers lie on, as `(start . end)`, in a text of `line-count` lines.
(define (md-lists/edited-spans edited line-count)
  (map (lambda (e)
         (let* ([hunk (car e)]
                [start (hash-ref hunk 'new-start-line)]
                [end (hash-ref hunk 'new-end-line)])
           (if (= start end)
               (cons (max 0 (- start 1)) (min line-count (+ end 1)))
               (cons start end))))
       edited))

;;; The events `edited` gives the list of `markers`. See docs/lists.md's "Renumbering after an edit".
(define (md-lists/list-events markers edited line-of line-info)
  (let* ([line-of-marker (lambda (m) (line-of (md-preview/capture-start m)))]
         [cols (map (lambda (m) (- (md-preview/capture-start m) (car (line-info (line-of-marker m))))) markers)])
    (call! "stdlib/concat"
      (map (lambda (e)
             (let* ([hunk (car e)]
                    [start (hash-ref hunk 'new-start-line)]
                    [end (hash-ref hunk 'new-end-line)]
                    [col (findf (lambda (c) (member c cols)) (cdr (cdr e)))]
                    [inside (filter (lambda (m) (let ([line (line-of-marker m)]) (and (<= start line) (< line end))))
                                    markers)]
                    [before (filter (lambda (m) (< (line-of-marker m) start)) markers)]
                    [after (filter (lambda (m) (>= (line-of-marker m) end)) markers)]
                    [old-numbers (if col (md-lists/numbers-at (car (cdr e)) col) '())])
               (cond
                 [(not col) '()]
                 [(pair? inside) (map (lambda (m) (cons m 'anchor)) inside)]
                 [(pair? before) (list (cons (last before) 'resume))]
                 [(and (pair? after) (pair? old-numbers)) (list (cons (car after) (cons 'start (car old-numbers))))]
                 [else '()])))
           edited))))

;;; The events `edited`, from `md-lists/edited-hunks`, gives the lists in `groups`.
(define (md-lists/edited-events groups edited line-of line-info)
  (call! "stdlib/concat"
    (map (lambda (markers) (md-lists/list-events markers edited line-of line-info))
         (map md-preview/children (md-preview/runs-of groups "list")))))

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
