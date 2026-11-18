;;; core:markdown — queries.scm. See README.md.

(provide md-preview/block-query md-preview/inline-query md-preview/atx-levels md-preview/setext-levels
         md-preview/capture-start md-preview/capture-end md-preview/line-span md-preview/inside?
         md-preview/line-slice md-preview/bucket md-preview/extent md-preview/records
         md-preview/conceal
         md-preview/conceal-before? md-preview/group md-preview/captured
         md-preview/heading md-preview/heading-text md-preview/setext md-preview/setext-text)

(define md-preview/atx-levels '(1 2 3 4 5 6))
(define md-preview/setext-levels '(1 2))

(define (md-preview/capture-start capture) (list-ref capture 1))
(define (md-preview/capture-end capture) (list-ref capture 2))

(define (md-preview/line-span line-of capture)
  (hash 'start-line (line-of (md-preview/capture-start capture))
        'end-line (+ (line-of (- (md-preview/capture-end capture) 1)) 1)))

(define (md-preview/inside? capture container)
  (and (>= (md-preview/capture-start capture) (md-preview/capture-start container))
       (<= (md-preview/capture-end capture) (md-preview/capture-end container))))

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

(define (md-preview/records containers items)
  (map cons containers
       (md-preview/bucket (map md-preview/extent containers) items md-preview/capture-start)))

(define (md-preview/line-slice start end line-of line-info)
  (let ([info (line-info (line-of start))])
    (substring (cdr info) (- start (car info)) (- end (car info)))))

(define (md-preview/conceal start end text scope reveal)
  (let* ([c (hash 'start-char start 'end-char end)]
         [c (if text (hash-insert c 'text text) c)]
         [c (if scope (hash-insert c 'scope scope) c)])
    (if reveal (hash-insert c 'reveal reveal) c)))

(define (md-preview/conceal-before? a b)
  (let ([a-start (hash-ref a 'start-char)] [b-start (hash-ref b 'start-char)])
    (or (< a-start b-start)
        (and (= a-start b-start) (> (hash-ref a 'end-char) (hash-ref b 'end-char))))))

(define (md-preview/group captures)
  (foldl (lambda (c groups)
           (hash-insert groups (car c) (cons c (or (hash-try-get groups (car c)) '()))))
         (hash)
         (reverse captures)))

(define (md-preview/captured groups name)
  (or (hash-try-get groups name) '()))

(define (md-preview/heading n) (string-append "heading." (number->string n)))
(define (md-preview/heading-text n) (string-append "heading-text." (number->string n)))
(define (md-preview/setext n) (string-append "setext." (number->string n)))
(define (md-preview/setext-text n) (string-append "setext-text." (number->string n)))

(define (md-preview/atx-patterns n)
  (let ([marker (string-append "(atx_h" (number->string n) "_marker)")])
    (string-append
      "(atx_heading " marker " @" (md-preview/heading n) ")\n"
      "(atx_heading " marker " (inline) @" (md-preview/heading-text n) ")\n")))

(define (md-preview/setext-patterns n)
  (let ([underline (string-append "(setext_h" (number->string n) "_underline)")])
    (string-append
      "(setext_heading " underline ") @" (md-preview/setext n) "\n"
      "(setext_heading (paragraph (inline) @" (md-preview/setext-text n) ") " underline ")\n")))

(define md-preview/block-query
  (string-append
    (apply string-append (map md-preview/atx-patterns md-preview/atx-levels))
    (apply string-append (map md-preview/setext-patterns md-preview/setext-levels))
    "[(list_marker_minus) (list_marker_plus) (list_marker_star)] @bullet\n"
    "(task_list_marker_unchecked) @task-unchecked\n"
    "(task_list_marker_checked) @task-checked\n"
    "(block_quote) @quote\n"
    "(block_quote_marker) @quote-marker\n"
    "(block_continuation) @continuation\n"
    "(code_fence_content (block_continuation) @fence-continuation)\n"
    "(thematic_break) @rule\n"
    "(fenced_code_block) @fence\n"
    "(fenced_code_block [(fenced_code_block_delimiter) (info_string)] @fence-hidden)\n"
    "(fenced_code_block (code_fence_content) @fence-content)\n"
    "(pipe_table) @table\n"
    "(pipe_table [(pipe_table_header) (pipe_table_row)] @table-row)\n"
    "(pipe_table (pipe_table_delimiter_row) @table-delimiter-row)\n"
    "(pipe_table [(pipe_table_header (pipe_table_cell) @table-cell)\n"
    "             (pipe_table_row (pipe_table_cell) @table-cell)])\n"
    "(pipe_table (pipe_table_delimiter_row (pipe_table_delimiter_cell) @table-delimiter-cell))\n"))

(define md-preview/inline-query
  (string-append
    "[(emphasis) (strong_emphasis) (strikethrough) (code_span)] @span\n"
    "(emphasis (emphasis_delimiter) @span-delimiter)\n"
    "(strong_emphasis (emphasis_delimiter) @span-delimiter)\n"
    "(strikethrough (emphasis_delimiter) @span-delimiter)\n"
    "(code_span (code_span_delimiter) @span-delimiter)\n"
    "(inline_link (link_text) @link-text) @link\n"
    "(full_reference_link (link_text) @link-text) @link\n"
    "(collapsed_reference_link (link_text) @link-text) @link\n"
    "(image (image_description) @link-text) @link\n"
    "[(uri_autolink) (email_autolink)] @autolink\n"
    "(backslash_escape) @escape\n"
    "[(entity_reference) (numeric_character_reference)] @entity\n"))
