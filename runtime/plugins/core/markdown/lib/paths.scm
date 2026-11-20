;;; core:markdown — paths.scm. See docs/links.md.

(require "destinations.scm")

(provide md-paths/complete!)

(define md-paths/file-kind 17)

(define md-paths/folder-kind 19)

(define (md-paths/dir-end prefix)
  (let loop ([i (string-length prefix)])
    (cond [(= i 0) 0]
          [(char=? (string-ref prefix (- i 1)) #\/) i]
          [else (loop (- i 1))])))

(define (md-paths/destination-before before)
  (let ([open (md-paths/link-open before)])
    (and open
         (let* ([angle? (and (< open (string-length before)) (char=? (string-ref before open) #\<))]
                [column (if angle? (+ open 1) open)]
                [typed (substring before column (string-length before))])
           (and (if angle? (md-destinations/angle-safe? typed) (md-destinations/bare-depth typed))
                (not (starts-with? typed "/"))
                (not (string-contains? typed "#"))
                (list column typed angle?))))))

(define (md-paths/link-open line)
  (let loop ([chars (reverse (string->list line))] [i (string-length line)])
    (cond [(or (null? chars) (null? (cdr chars))) #f]
          [(and (char=? (car chars) #\() (char=? (cadr chars) #\])) i]
          [else (loop (cdr chars) (- i 1))])))

(define (md-paths/new-text path folder? form)
  (cond [(not (md-destinations/angle-safe? path)) #f]
        [(eq? form 'bare) (cond [(not folder?) (md-destinations/encode path)]
                                [(md-destinations/bare? path) path]
                                [else (string-append "<" path)])]
        [folder? path]
        [else (string-append path ">")]))

(define (md-paths/item entry dir-part form edit)
  (let* ([folder? (ends-with? entry "/")]
         [path (string-append dir-part (if folder? (trim-end-matches entry "/") entry))]
         [new-text (md-paths/new-text path folder? form)])
    (and new-text
         (hash "label" entry
               "filterText" path
               "kind" (if folder? md-paths/folder-kind md-paths/file-kind)
               "textEdit" (edit folder? new-text)))))

(define (md-paths/answer pane)
  (let* ([head (call! "stdlib/cursor-char-offset" (buffer-selections pane))]
         [line (and head (char-offset->line pane head))]
         [line-start (and line (line->char-offset pane line))]
         [found (and line
                     (md-paths/destination-before
                       (buffer-text pane #:range (hash 'start-char line-start 'end-char head))))]
         [typed (and found (cadr found))]
         [cut (and found (md-paths/dir-end typed))]
         [dir-part (and found (substring typed 0 cut))]
         [folder (and found (md-destinations/resolve pane dir-part))])
    (and folder
         (let* ([name (substring typed cut (string-length typed))]
                [form (cond [(not (caddr found)) 'bare]
                            [(equal? (buffer-text pane #:range (hash 'start-char head
                                                                     'end-char (+ head 1)))
                                     ">")
                             'closed]
                            [else 'open])]
                [end (- head line-start)]
                [edit (lambda (folder? new-text)
                        (hash "range"
                              (hash "start" (hash "line" line "character" (car found))
                                    "end" (hash "line" line
                                                "character" (if (and (eq? form 'closed) (not folder?))
                                                                (+ end 1)
                                                                end)))
                              "newText" new-text))])
           (cons (filter (lambda (item) item)
                         (map (lambda (entry) (md-paths/item entry dir-part form edit))
                              (path-entries folder (if (starts-with? name ".") "." ""))))
                 (equal? name ""))))))

;;; See docs/links.md's "Link paths".
(define (md-paths/complete! id pane prefix)
  (let ([answer (md-paths/answer pane)])
    (if answer
        (completion-emit! id (car answer) #:incomplete (cdr answer))
        (completion-emit! id '()))))
