;;; core:markdown — paths.scm. See docs/links.md.

(require "destinations.scm")

(provide md-paths/complete!)

(define md-paths/file-kind 17)

(define md-paths/folder-kind 19)

(struct md-paths/destination (column dir-part name angle?))

(define (md-paths/destination-before before)
  (let ([parts (split-many before "](")])
    (and (pair? (cdr parts))
         (let* ([after (last parts)]
                [angle? (starts-with? after "<")]
                [typed (if angle? (substring after 1 (string-length after)) after)])
           (and (if angle? (md-destinations/angle-safe? typed) (md-destinations/bare-depth typed))
                (not (starts-with? typed "/"))
                (not (string-contains? typed "#"))
                (let* ([name (last (split-many typed "/"))]
                       [typed-length (string-length typed)])
                  (md-paths/destination (- (string-length before) typed-length)
                                        (substring typed 0 (- typed-length (string-length name)))
                                        name
                                        angle?)))))))

(define (md-paths/item entry found range closing-range)
  (let* ([folder? (ends-with? entry "/")]
         [path (string-append (md-paths/destination-dir-part found)
                              (if folder? (trim-end-matches entry "/") entry))]
         [angle? (md-paths/destination-angle? found)]
         [wrapped? (or angle? (not (md-destinations/bare? path)))]
         [closes? (and wrapped? (not folder?))])
    (and (or (not wrapped?) (md-destinations/angle-safe? path))
         (hash "label" entry
               "filterText" path
               "kind" (if folder? md-paths/folder-kind md-paths/file-kind)
               "textEdit" (hash "range" (if closes? closing-range range)
                                "newText" (string-append (if (and wrapped? (not angle?)) "<" "")
                                                         path
                                                         (if closes? ">" "")))))))

(define (md-paths/answer pane)
  (let* ([head (call! "stdlib/cursor-char-offset" (buffer-selections pane))]
         [line (and head (char-offset->line pane head))]
         [line-start (and line (line->char-offset pane line))]
         [found (and line
                     (md-paths/destination-before
                       (buffer-text pane #:range (hash 'start-char line-start 'end-char head))))]
         [folder (and found (md-destinations/resolve pane (md-paths/destination-dir-part found)))])
    (and folder
         (let* ([name (md-paths/destination-name found)]
                [start (hash "line" line "character" (md-paths/destination-column found))]
                [end (- head line-start)]
                [range (hash "start" start "end" (hash "line" line "character" end))]
                [closing-range
                 (if (and (md-paths/destination-angle? found)
                          (equal? (buffer-text pane #:range (hash 'start-char head 'end-char (+ head 1)))
                                  ">"))
                     (hash "start" start "end" (hash "line" line "character" (+ end 1)))
                     range)])
           (cons (filter (lambda (item) item)
                         (map (lambda (entry) (md-paths/item entry found range closing-range))
                              (path-entries folder (if (starts-with? name ".") "." ""))))
                 (equal? name ""))))))

;;; See docs/links.md's "Link paths".
(define (md-paths/complete! id pane prefix)
  (let ([answer (md-paths/answer pane)])
    (if answer
        (completion-emit! id (car answer) #:incomplete (cdr answer))
        (completion-emit! id '()))))
