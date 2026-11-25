;;; core:markdown — queries.scm. See docs/elements.md.

(provide md-preview/block-query md-preview/inline-query md-preview/atx-levels md-preview/setext-levels
         md-preview/capture-start md-preview/capture-end md-preview/capture-before? md-preview/lines-of
         md-preview/range-span md-preview/line-span md-preview/inside? md-preview/innermost md-preview/line-slice
         md-preview/blank? md-preview/all? md-preview/skip-while md-preview/skip-blanks md-preview/blank-from?
         md-preview/edit md-preview/insert md-preview/char-start
         md-preview/quote-cols md-preview/bucket md-preview/extent
         md-preview/conceal md-preview/hidden-revealing
         md-preview/conceal-before? md-preview/shown-text md-preview/group md-preview/runs-of md-preview/runs md-preview/children
         md-preview/child md-preview/captured
         md-preview/group-by md-preview/unchecked-box md-preview/checked-box
         md-preview/start-col md-preview/end-col md-preview/capture-line md-preview/line-end
         md-preview/task-box md-preview/marker md-preview/line-slicer md-preview/level-capture
         md-lists/query md-lists/numbered-query)

(define md-preview/atx-levels '(1 2 3 4 5 6))
(define md-preview/setext-levels '(1 2))

(define (md-preview/capture-start capture) (list-ref capture 1))
(define (md-preview/capture-end capture) (list-ref capture 2))

(define (md-preview/capture-before? a b)
  (< (md-preview/capture-start a) (md-preview/capture-start b)))

;;; The lines chars `[start, end)` lie on, as `(first . end)` with `end` exclusive.
(define (md-preview/lines-of line-of start end)
  (cons (line-of start) (+ (line-of (- end 1)) 1)))

;;; The lines chars `[start, end)` lie on, as a range of `'start-line` and exclusive `'end-line`.
(define (md-preview/range-span line-of start end)
  (let ([lines (md-preview/lines-of line-of start end)])
    (hash 'start-line (car lines) 'end-line (cdr lines))))

(define (md-preview/line-span line-of capture)
  (md-preview/range-span line-of (md-preview/capture-start capture) (md-preview/capture-end capture)))

(define (md-preview/inside? capture container)
  (and (>= (md-preview/capture-start capture) (md-preview/capture-start container))
       (<= (md-preview/capture-end capture) (md-preview/capture-end container))))

;;; The item of `items` whose capture, `(capture-of item)`, is the innermost holding chars `[start, end)`, or `#f`.
(define (md-preview/innermost items start end capture-of)
  (foldl (lambda (item best)
           (let ([capture (capture-of item)])
             (if (and (<= (md-preview/capture-start capture) start)
                      (<= end (md-preview/capture-end capture))
                      (or (not best) (md-preview/inside? capture (capture-of best))))
                 item
                 best)))
         #f
         items))

(define (md-preview/blank? c) (or (char=? c #\space) (char=? c #\tab)))

(define (md-preview/all? pred? items) (not (findf (lambda (item) (not (pred? item))) items)))

(define (md-preview/skip-while text col keep?)
  (if (and (< col (string-length text)) (keep? (string-ref text col)))
      (md-preview/skip-while text (+ col 1) keep?)
      col))

(define (md-preview/skip-blanks text col)
  (md-preview/skip-while text col md-preview/blank?))

;;; Whether `text` holds only blanks from `col` on.
(define (md-preview/blank-from? text col)
  (= (md-preview/skip-blanks text col) (string-length text)))

;;; An edit replacing chars `[start, end)` with `text`.
(define (md-preview/edit start end text) (hash 'start-char start 'end-char end 'text text))

(define (md-preview/insert at text) (md-preview/edit at at text))

(define (md-preview/char-start conceal) (hash-ref conceal 'start-char))

;;; The column of each `>` in `capture`, which lies on the line `text` starting at `start`, in order, before `tail`.
(define (md-preview/quote-cols capture start text tail)
  (let ([from (md-preview/start-col capture start)])
    (let loop ([col (- (min (string-length text) (- (md-preview/capture-end capture) start)) 1)]
               [cols tail])
      (cond
        [(< col from) cols]
        [(char=? (string-ref text col) #\>) (loop (- col 1) (cons col cols))]
        [else (loop (- col 1) cols)]))))

(define (md-preview/bucket spans items item-start)
  (let loop ([spans spans] [items items] [buckets '()])
    (cond
      [(null? spans) (reverse buckets)]
      [(and (pair? items) (< (item-start (car items)) (car (car spans))))
       (loop spans (cdr items) buckets)]
      [else
       (let take ([items items] [taken '()])
         (if (and (pair? items) (<= (item-start (car items)) (cdr (car spans))))
             (take (cdr items) (cons (car items) taken))
             (loop (cdr spans) items (cons (reverse taken) buckets))))])))

(define (md-preview/extent capture)
  (cons (md-preview/capture-start capture) (- (md-preview/capture-end capture) 1)))

;;; Slices the line `info` reads, `(start . text)`, by buffer offsets.
(define (md-preview/line-slicer info)
  (lambda (start end) (substring (cdr info) (- start (car info)) (- end (car info)))))

(define (md-preview/line-end info)
  (+ (car info) (string-length (cdr info))))

;;; The line `capture` starts on, as `line-info` reads it.
(define (md-preview/capture-line capture line-of line-info)
  (line-info (line-of (md-preview/capture-start capture))))

(define (md-preview/line-slice start end line-of line-info)
  ((md-preview/line-slicer (line-info (line-of start))) start end))

(define (md-preview/conceal start end text scope reveal)
  (let* ([c (hash 'start-char start 'end-char end)]
         [c (if text (hash-insert c 'text text) c)]
         [c (if scope (hash-insert c 'scope scope) c)])
    (if reveal (hash-insert c 'reveal reveal) c)))

(define (md-preview/hidden-revealing captures reveal)
  (map (lambda (c) (md-preview/conceal (md-preview/capture-start c) (md-preview/capture-end c) #f #f reveal))
       captures))

(define (md-preview/conceal-before? a b)
  (let ([a-start (hash-ref a 'start-char)] [b-start (hash-ref b 'start-char)])
    (or (< a-start b-start)
        (and (= a-start b-start) (> (hash-ref a 'end-char) (hash-ref b 'end-char))))))

;;; The chars `span`, `(start . end)`, as the preview shows them once `conceals`, sorted by
;;; `md-preview/conceal-before?`, apply. `slice` reads the text between two offsets.
(define (md-preview/shown-text span conceals slice)
  (let ([end (cdr span)])
    (let loop ([cs conceals] [at (car span)] [out '()])
      (let ([from (and (pair? cs) (hash-ref (car cs) 'start-char))])
        (cond
          [(or (not from) (>= from end)) (apply string-append (reverse (cons (slice at end) out)))]
          [(< from at) (loop (cdr cs) at out)]
          [else (loop (cdr cs)
                      (min end (hash-ref (car cs) 'end-char))
                      (cons (or (hash-try-get (car cs) 'text) "") (cons (slice at from) out)))])))))

;;; `items` in a hash by `(key item)`, each key's items in order.
(define (md-preview/group-by key items)
  (foldl (lambda (item groups)
           (let ([k (key item)])
             (hash-insert groups k (cons item (or (hash-try-get groups k) '())))))
         (hash)
         (reverse items)))

(define (md-preview/group matches)
  (md-preview/group-by (lambda (run) (car (car run))) (md-preview/runs matches)))

(define (md-preview/runs-of groups name)
  (or (hash-try-get groups name) '()))

;;; Per run of adjacent matches with one first entry, `(head . tails)`: that entry, and each match's other entries.
(define (md-preview/runs matches)
  (foldl (lambda (m runs)
           (if (and (pair? runs) (equal? (car (car runs)) (car m)))
               (cons (cons (car m) (cons (cdr m) (cdr (car runs)))) (cdr runs))
               (cons (list (car m) (cdr m)) runs)))
         '()
         (reverse matches)))

;;; The second entry of each match in `run`.
(define (md-preview/children run) (map car (cdr run)))

;;; The second entry of the first match in `run` that has one, or `#f`.
(define (md-preview/child run)
  (let ([tail (findf pair? (cdr run))]) (and tail (car tail))))

(define (md-preview/captured groups name)
  (map car (md-preview/runs-of groups name)))

(define (md-preview/level-capture kind n) (string-append kind "." (number->string n)))

(define (md-preview/atx-patterns n)
  (let ([marker (string-append "(atx_h" (number->string n) "_marker)")])
    (string-append
      "(atx_heading " marker " @" (md-preview/level-capture "heading" n) ")\n"
      "(atx_heading " marker " (inline) @" (md-preview/level-capture "heading-text" n) ")\n")))

(define (md-preview/setext-patterns n)
  (let ([underline (string-append "(setext_h" (number->string n) "_underline)")])
    (string-append
      "(setext_heading " underline ") @" (md-preview/level-capture "setext" n) "\n"
      "(setext_heading (paragraph (inline) @" (md-preview/level-capture "setext-text" n) ") " underline ")\n")))

(define md-preview/unchecked-box "task-unchecked")
(define md-preview/checked-box "task-checked")

;;; The column where `capture` starts on a line starting at `start`.
(define (md-preview/start-col capture start)
  (- (md-preview/capture-start capture) start))

;;; The column where `capture` ends on the line `text` starting at `start`, at most the text's length.
(define (md-preview/end-col capture start text)
  (min (string-length text) (- (md-preview/capture-end capture) start)))

;;; The list marker capture `marker`, on the line `text` starting at `start`, from its first non-blank.
;;; See docs/elements.md's "Node shapes".
(define (md-preview/marker marker start text)
  (cons (car marker)
        (cons (+ start (md-preview/skip-blanks text (md-preview/start-col marker start))) (cddr marker))))

;;; The task box after `marker` on the line `text` starting at `start`: `parsed`, the parser's,
;;; or else one read from the line. See docs/elements.md's "Node shapes".
(define (md-preview/task-box marker parsed start text)
  (or parsed
      (let* ([from (md-preview/skip-blanks text (md-preview/end-col marker start text))]
             [to (+ from 3)])
        (and (<= to (string-length text))
             (char=? (string-ref text from) #\[)
             (char=? (string-ref text (+ from 2)) #\])
             (md-preview/blank-from? text to)
             (let ([mark (string-ref text (+ from 1))])
               (cond
                 [(md-preview/blank? mark) (list md-preview/unchecked-box (+ start from) (+ start to))]
                 [(member mark '(#\x #\X)) (list md-preview/checked-box (+ start from) (+ start to))]
                 [else #f]))))))

(define md-preview/bullet-markers "[(list_marker_minus) (list_marker_plus) (list_marker_star)]")
(define md-preview/ordered-markers "[(list_marker_dot) (list_marker_parenthesis)]")

(define (md-preview/item-pattern markers name)
  (string-append
    "(list_item " markers " @" name "\n"
    "           . [(task_list_marker_unchecked) @" md-preview/unchecked-box
    " (task_list_marker_checked) @" md-preview/checked-box "]?)\n"))

(define md-preview/block-query
  (string-append
    (apply string-append (map md-preview/atx-patterns md-preview/atx-levels))
    (apply string-append (map md-preview/setext-patterns md-preview/setext-levels))
    (md-preview/item-pattern md-preview/bullet-markers "bullet")
    "(block_quote) @quote\n"
    "(block_quote_marker) @quote-marker\n"
    "(block_continuation) @continuation\n"
    "(code_fence_content (block_continuation) @fence-continuation)\n"
    "(thematic_break) @rule\n"
    "(fenced_code_block [(fenced_code_block_delimiter) (info_string)] @fence-hidden) @fence\n"
    "(fenced_code_block (code_fence_content) @fence-content)\n"
    "(pipe_table [(pipe_table_header (pipe_table_cell) @table-cell)\n"
    "             (pipe_table_row (pipe_table_cell) @table-cell)] @table-row) @table\n"
    "(pipe_table (pipe_table_delimiter_row (pipe_table_delimiter_cell) @table-delimiter-cell)\n"
    "            @table-delimiter-row) @table\n"))

(define md-preview/inline-query
  (string-append
    "(emphasis (emphasis_delimiter) @span-delimiter) @span\n"
    "(strong_emphasis (emphasis_delimiter) @span-delimiter) @span\n"
    "(strikethrough (emphasis_delimiter) @span-delimiter) @span\n"
    "(code_span (code_span_delimiter) @span-delimiter) @span\n"
    "(inline_link (link_text) @link-text) @link\n"
    "(full_reference_link (link_text) @link-text) @link\n"
    "(collapsed_reference_link (link_text) @link-text) @link\n"
    "(image (image_description) @link-text) @link\n"
    "[(uri_autolink) (email_autolink)] @autolink\n"
    "(backslash_escape) @escape\n"
    "[(entity_reference) (numeric_character_reference)] @entity\n"))

(define md-lists/numbered-query
  (string-append "(list (list_item " md-preview/ordered-markers " @ordered)) @list\n"
                 "(list_item [" md-preview/bullet-markers " " md-preview/ordered-markers "] @list-parent (list) @nested)\n"))

(define md-lists/query
  (string-append
    (md-preview/item-pattern md-preview/bullet-markers "bullet")
    (md-preview/item-pattern md-preview/ordered-markers "ordered")
    md-lists/numbered-query
    "[(block_quote_marker) (block_continuation)] @quote-prefix\n"))
