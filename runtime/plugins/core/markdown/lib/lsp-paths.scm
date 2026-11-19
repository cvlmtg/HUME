;;; core:markdown — lsp-paths.scm. See docs/links.md.

(provide md-lsp-paths/rewrite)

(define (md-lsp-paths/path-start text)
  (if (starts-with? text "/")
      0
      (let loop ([i 0])
        (cond [(> (+ i 3) (string-length text)) #f]
              [(equal? (substring text i (+ i 3)) "](/") (+ i 2)]
              [else (loop (+ i 1))]))))

(define (md-lsp-paths/path-end text start)
  (let loop ([i start])
    (cond [(= i (string-length text)) i]
          [(memv (string-ref text i) '(#\# #\) #\space)) i]
          [else (loop (+ i 1))])))

(define (md-lsp-paths/root server path)
  (let ([name (lsp-server-name server)]
        [sep (path-separator)])
    (foldl (lambda (entry best)
             (let ([root (hash-ref entry 'root)])
               (if (and (equal? (hash-ref entry 'name) name)
                        (starts-with? path (string-append root sep))
                        (or (not best) (> (string-length root) (string-length best))))
                   root
                   best)))
           #f
           (lsp-server-status))))

(define (md-lsp-paths/relative dir root bare)
  (let ([sep (path-separator)])
    (let loop ([up (if (equal? dir root)
                       '()
                       (split-many (substring dir (+ (string-length root) (string-length sep))
                                              (string-length dir))
                                   sep))]
               [down (split-many bare "/")])
      (if (and (pair? up) (pair? (cdr down)) (equal? (car up) (car down)))
          (loop (cdr up) (cdr down))
          (string-append (apply string-append (map (lambda (_) "../") up))
                         (string-join down "/"))))))

(define (md-lsp-paths/item-edits item at dir root)
  (let* ([written (json-ref-or item #f "textEdit" "newText")]
         [start (and (string? written) (md-lsp-paths/path-start written))]
         [end (and start (md-lsp-paths/path-end written start))]
         [rooted (and start (substring written start end))]
         [bare (and rooted (substring rooted 1 (string-length rooted)))]
         [relative (and bare (md-lsp-paths/relative dir root bare))])
    (if relative
        (let ([rewritten (string-append (substring written 0 start)
                                        relative
                                        (substring written end (string-length written)))]
              [filter (json-ref-or item #f "filterText")]
              [label (json-ref item "label")])
          (append
            (list (cons (append at '("textEdit" "newText")) rewritten)
                  (cons (append at '("filterText"))
                        (if (string? filter) (string-replace filter rooted relative) rewritten)))
            (if (starts-with? label bare)
                (list (cons (append at '("label"))
                            (string-append relative
                                           (substring label (string-length bare) (string-length label)))))
                '())))
        '())))

;;; See docs/links.md's "Language server paths".
(define (md-lsp-paths/rewrite pane server answer)
  (let* ([path (buffer-path pane)]
         [root (and path (md-lsp-paths/root server path))]
         [at (if (json-array? answer) '() '("items"))])
    (if (and path root (or (json-array? answer) (json-contains? answer "items")))
        (let* ([items (json-list (if (null? at) answer (json-ref answer "items")))]
               [edits (apply append
                             (map (lambda (item i)
                                    (if (json-object? item)
                                        (md-lsp-paths/item-edits item (append at (list i))
                                                                 (parent-name path) root)
                                        '()))
                                  items
                                  (range 0 (length items))))])
          (if (null? edits) answer (json-with answer edits)))
        answer)))
