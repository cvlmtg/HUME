;;; core:markdown — lists.scm. See docs/lists.md.

(require "queries.scm")
(require "state.scm")

(provide md-lists/distinct md-lists/may-start-prefix? md-lists/items md-lists/past-prefix? md-lists/empty?
         md-lists/ended-edit
         md-lists/events md-lists/numbered-lists md-lists/renumber md-lists/prefixes
         md-lists/edited-hunks md-lists/edited-spans md-lists/edited-events md-lists/task-edits)

(define (md-lists/distinct lines)
  (reverse (foldl (lambda (line kept) (if (and (pair? kept) (= (car kept) line)) kept (cons line kept)))
                  '()
                  lines)))

(define (md-lists/token-length text col)
  (- (md-preview/skip-while text col (lambda (c) (not (md-preview/blank? c)))) col))

(define (md-lists/past-bars bars text)
  (md-preview/skip-blanks text (+ (apply max bars) 1)))

;;; Whether `text` starts, past blanks, with a character a list or quote marker starts with: every
;;; line with a prefix does.
(define (md-lists/may-start-prefix? text)
  (let ([col (md-preview/skip-blanks text 0)])
    (and (< col (string-length text))
         (let ([c (string-ref text col)])
           (or (char-digit? c) (and (memv c '(#\> #\- #\* #\+)) #t))))))

;;; See docs/lists.md's "Prefixes".
(define (md-lists/item info marked quote-prefixes)
  (let* ([start (car info)]
         [text (cdr info)]
         [bars (foldl (lambda (c tail) (md-preview/quote-cols c start text tail))
                      '()
                      (reverse quote-prefixes))])
    (and (or (pair? marked) (pair? bars))
         (let ([item (hash 'start start 'text text 'markers (map car marked) 'bars bars 'marker #f 'task #f)])
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
               (hash-insert item 'prefix-end (md-lists/past-bars bars text)))))))

;;; Per line of `lines`, ascending with repeats allowed, its item or `#f`.
(define (md-lists/items lines groups line-info)
  (let* ([unique (md-lists/distinct lines)]
         [infos (map line-info unique)]
         [spans (map (lambda (info) (cons (car info) (md-preview/line-end info))) infos)]
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
      (let ([from (md-preview/start-col capture start)])
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
                        (cons (substring text col (md-preview/start-col (car captures) start)) parts))))))))

;;; See docs/lists.md's "Ending a list".
(define (md-lists/kept-col item)
  (let ([text (hash-ref item 'text)]
        [markers (hash-ref item 'markers)]
        [bars (hash-ref item 'bars)])
    (if (pair? markers)
        (let* ([cut (md-preview/start-col (car markers) (hash-ref item 'start))]
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

;;; `start` is the number a `'start` event gives its marker, `#f` for any other kind.
(struct md-lists/event (marker kind start))

;;; An ordered marker of a list: its capture, the line it is on and its column there, its
;;; number, how many digits that number has, and its delimiter.
(struct md-lists/numbered (capture line col number width delimiter))

(define (md-lists/numbered-start n) (md-preview/capture-start (md-lists/numbered-capture n)))

;;; Per list in `groups`, its ordered markers in order. `(read-lines start end)` gives lines
;;; `[start, end)` as `(start-char . texts)`. See docs/lists.md's "Renumbering".
(define (md-lists/numbered-lists groups line-of read-lines)
  (define (numbered captures)
    (let* ([first-line (line-of (md-preview/capture-start (car captures)))]
           [read (read-lines first-line (+ (line-of (md-preview/capture-start (last captures))) 1))])
      (let loop ([captures captures] [line first-line] [start (car read)] [texts (cdr read)] [out '()])
        (if (null? captures)
            (reverse out)
            (let ([next-start (+ start (string-length (car texts)) 1)]
                  [capture (car captures)])
              (if (>= (md-preview/capture-start capture) next-start)
                  (loop captures (+ line 1) next-start (cdr texts) out)
                  (let* ([text (car texts)]
                         [col (md-preview/start-col capture start)]
                         [end (md-preview/skip-while text col char-digit?)])
                    (loop (cdr captures) line start texts
                          (cons (md-lists/numbered capture line col (string->number (substring text col end))
                                                   (- end col) (string-ref text end))
                                out)))))))))
  (map numbered (map md-preview/children (md-preview/runs-of groups "list"))))

;;; See docs/lists.md's "Renumbering".
(define (md-lists/renumber lists events)
  (let* ([indexed (map cons (range 0 (length events)) events)]
         [marker-of (lambda (e) (md-lists/event-marker (cdr e)))]
         [numbered (filter (lambda (e) (let ([m (marker-of e)]) (and m (equal? (car m) "ordered"))))
                           indexed)]
         [kind-of (lambda (e) (md-lists/event-kind (cdr e)))]
         [start-of (lambda (e) (md-lists/event-start (cdr e)))]
         [at-marker (md-preview/group-by (lambda (e) (md-preview/capture-start (marker-of e))) numbered)]
         [numbers (box (hash))]
         [edits (box '())])
    (define number md-lists/numbered-number)
    (define (events-at n) (or (hash-try-get at-marker (md-lists/numbered-start n)) '()))
    (define (of-kind at k) (filter (lambda (e) (equal? (kind-of e) k)) at))
    (define (uniform items)
      (let loop ([items items] [same #f] [count 0])
        (cond
          [(null? items) (and (>= count 2) same)]
          [(pair? (of-kind (events-at (car items)) 'anchor)) (loop (cdr items) same count)]
          [else (let ([n (number (car items))])
                  (and (or (not same) (= n same))
                       (loop (cdr items) n (+ count 1))))])))
    (define (assign! es counter)
      (foldl (lambda (e n)
               (md-preview/box-insert! numbers (car e) n)
               (+ n 1))
             counter
             es))
    (define (renumber! n counter)
      (unless (= counter (number n))
        (let ([s (md-lists/numbered-start n)])
          (set-box! edits
                    (cons (hash 'start-char s
                                'end-char (+ s (md-lists/numbered-width n))
                                'text (number->string counter))
                          (unbox edits))))))
    (for-each
      (lambda (items)
        (let ([same (uniform items)])
          (if same
              (for-each (lambda (n)
                          (let ([at (events-at n)])
                            (for-each (lambda (e) (md-preview/box-insert! numbers (car e) same))
                                      (append (of-kind at 'before) (of-kind at 'after)))))
                        items)
              (let walk ([items items] [counter #f])
                (unless (null? items)
                  (let* ([n (car items)]
                         [at (events-at n)]
                         [befores (of-kind at 'before)]
                         [starts (of-kind at 'start)]
                         [afters (of-kind at 'after)]
                         [counter (if (pair? befores) (assign! befores (or counter (number n))) counter)]
                         [counter (cond
                                    [(pair? (of-kind at 'anchor)) (+ (number n) 1)]
                                    [(pair? starts)
                                     (let ([start (start-of (car starts))]) (renumber! n start) (+ start 1))]
                                    [counter (renumber! n counter) (+ counter 1)]
                                    [(pair? (of-kind at 'resume)) (+ (number n) 1)]
                                    [else #f])]
                         [counter (if (pair? afters) (assign! afters (or counter (+ (number n) 1))) counter)])
                    (walk (cdr items) counter)))))))
      (filter (lambda (items) (findf (lambda (n) (hash-contains? at-marker (md-lists/numbered-start n))) items))
              lists))
    (cons (unbox edits)
          (map (lambda (e) (hash-try-get (unbox numbers) (car e))) indexed))))

;;; One event of `kind` per item, on its marker, or on `#f` for a line with no prefix.
(define (md-lists/events items kind)
  (map (lambda (item) (md-lists/event (and item (hash-ref item 'marker)) kind #f)) items))

;;; An ordered marker a line's text holds, `delimiter` being `#\.` or `#\)`.
(struct md-lists/line-marker (col number delimiter))

;;; The ordered markers in `text`'s container prefix.
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
                 (loop (+ end 1)
                       (cons (md-lists/line-marker col (string->number (substring text col end)) (string-ref text end))
                             out))]
                [else (reverse out)])))))))

(define (md-lists/numbers-at markers col)
  (map md-lists/line-marker-number (filter (lambda (m) (= (md-lists/line-marker-col m) col)) markers)))

;;; The ordered markers of lines `[start, end)` of `by-line`, a vector of each line's markers.
(define (md-lists/markers-in by-line start end)
  (call! "stdlib/concat" (map (lambda (line) (vector-ref by-line line)) (range start end))))

;;; The columns at which `old` and `new`, the ordered markers of two runs of lines, hold different numbers.
(define (md-lists/changed-cols old new)
  (filter (lambda (col) (not (equal? (md-lists/numbers-at old col) (md-lists/numbers-at new col))))
          (md-lists/distinct (sort (map md-lists/line-marker-col (append old new)) <))))

;;; A change to ordered markers: its new lines `[start, end)`, the markers of its old lines, and the
;;; columns whose numbers it changed.
(struct md-lists/edited (start end old cols))

;;; The changes `hunks` made to the numbers their lines' ordered markers hold, each hunk split into
;;; the lines it changed. See docs/lists.md's "Renumbering after an edit".
(define (md-lists/edited-hunks hunks)
  (define (side-text lines) (apply string-append (map (lambda (line) (string-append line "\n")) lines)))
  (define (edited hunk)
    (let* ([old-lines (hash-ref hunk 'old-lines)]
           [new-lines (hash-ref hunk 'new-lines)]
           [old-by-line (list->vector (map md-lists/line-markers old-lines))]
           [new-by-line (list->vector (map md-lists/line-markers new-lines))]
           [offset (hash-ref hunk 'new-start-line)])
      ;; `part`'s line numbers count from the hunk's first line on each side.
      (define (part-edit part)
        (let ([old (md-lists/markers-in old-by-line (hash-ref part 'old-start-line) (hash-ref part 'old-end-line))]
              [new (md-lists/markers-in new-by-line (hash-ref part 'new-start-line) (hash-ref part 'new-end-line))])
          (md-lists/edited (+ offset (hash-ref part 'new-start-line))
                           (+ offset (hash-ref part 'new-end-line))
                           old
                           (md-lists/changed-cols old new))))
      (cond
        [(null? (md-lists/changed-cols (md-lists/markers-in old-by-line 0 (length old-lines))
                                       (md-lists/markers-in new-by-line 0 (length new-lines))))
         '()]
        [(or (null? old-lines) (null? new-lines))
         (list (part-edit (hash 'old-start-line 0 'old-end-line (length old-lines)
                                'new-start-line 0 'new-end-line (length new-lines))))]
        [else
         (filter (lambda (e) (pair? (md-lists/edited-cols e)))
                 (map part-edit (hash-ref (diff-lines (side-text old-lines) (side-text new-lines)) 'hunks)))])))
  (call! "stdlib/concat" (map edited hunks)))

;;; The line ranges the lists `edited` renumbers lie on, as `(start . end)`, in a text of `line-count` lines:
;;; each hunk's new lines, from the last non-blank line before them through the first one after them.
(define (md-lists/edited-spans edited line-count line-info)
  (define (blank? line)
    (let ([text (cdr (line-info line))])
      (= (md-preview/skip-blanks text 0) (string-length text))))
  (map (lambda (e)
         (cons (let back ([line (- (md-lists/edited-start e) 1)])
                 (if (and (> line 0) (blank? line)) (back (- line 1)) (max line 0)))
               (let forward ([line (md-lists/edited-end e)])
                 (cond
                   [(>= line line-count) line-count]
                   [(blank? line) (forward (+ line 1))]
                   [else (+ line 1)]))))
       edited))

;;; The events `edited` gives the list `numbered`. See docs/lists.md's "Renumbering after an edit".
(define (md-lists/list-events numbered edited)
  (let* ([cols (map md-lists/numbered-col numbered)]
         [markers-where (lambda (keep?)
                          (map md-lists/numbered-capture
                               (filter (lambda (n) (keep? (md-lists/numbered-line n))) numbered)))]
         [delimiter (md-lists/numbered-delimiter (car numbered))]
         [delimited (lambda (old) (filter (lambda (m) (char=? (md-lists/line-marker-delimiter m) delimiter)) old))])
    (call! "stdlib/concat"
      (map (lambda (e)
             (let ([col (findf (lambda (c) (member c cols)) (md-lists/edited-cols e))])
               (if col
                   (let* ([start (md-lists/edited-start e)]
                          [end (md-lists/edited-end e)]
                          [inside (markers-where (lambda (line) (and (<= start line) (< line end))))]
                          [before (markers-where (lambda (line) (< line start)))]
                          [after (markers-where (lambda (line) (>= line end)))]
                          [old-numbers (md-lists/numbers-at (delimited (md-lists/edited-old e)) col)])
                     (cond
                       [(pair? inside) (map (lambda (m) (md-lists/event m 'anchor #f)) inside)]
                       [(pair? before) (list (md-lists/event (last before) 'resume #f))]
                       [(and (pair? after) (pair? old-numbers))
                        (list (md-lists/event (car after) 'start (car old-numbers)))]
                       [else '()]))
                   '())))
           edited))))

;;; The events `edited`, from `md-lists/edited-hunks`, gives `lists`, from `md-lists/numbered-lists`.
(define (md-lists/edited-events lists edited)
  (call! "stdlib/concat" (map (lambda (numbered) (md-lists/list-events numbered edited)) lists)))

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
