;;; core:markdown — paths.scm. See docs/links.md.

(provide md-paths/complete!)

(define md-paths/file-kind 17)

(define md-paths/folder-kind 19)

(define (md-paths/dir-end prefix)
  (let loop ([i (string-length prefix)])
    (cond [(= i 0) 0]
          [(char=? (string-ref prefix (- i 1)) #\/) i]
          [else (loop (- i 1))])))

(define (md-paths/in-destination? pane prefix)
  (let* ([head (call! "stdlib/cursor-char-offset" (buffer-selections pane))]
         [start (and head (- head (string-length prefix)))])
    (and start
         (>= start 2)
         (equal? (buffer-text pane #:range (hash 'start-char (- start 2) 'end-char start)) "]("))))

;;; See docs/links.md's "Link paths".
(define (md-paths/complete! id pane prefix)
  (let ([path (buffer-path pane)])
    (if (and path
             (not (starts-with? prefix "/"))
             (md-paths/in-destination? pane prefix))
        (let* ([cut (md-paths/dir-end prefix)]
               [dir-part (substring prefix 0 cut)]
               [name (substring prefix cut (string-length prefix))]
               [names (path-entries (path-join (parent-name path) dir-part)
                                    (if (starts-with? name ".") "." ""))])
          (completion-emit!
            id
            (map (lambda (entry)
                   (let* ([folder? (ends-with? entry "/")]
                          [target (string-append dir-part
                                                 (if folder?
                                                     (substring entry 0 (- (string-length entry) 1))
                                                     entry))])
                     (hash "label" entry
                           "insertText" target
                           "filterText" target
                           "kind" (if folder? md-paths/folder-kind md-paths/file-kind))))
                 names)
            #:incomplete (equal? name "")))
        (completion-emit! id '()))))
