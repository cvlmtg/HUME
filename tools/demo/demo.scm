;;; demo plugin — inserts a markdown table of contents

;; The characters VitePress turns into `-` when it builds a heading's anchor.
(define demo/slug-separators (string->list "~`!@#$%^&*()-_+=[]{}|\\;:\"'“”‘’<>,.?/"))

(define (demo/separator? c)
  (or (char-whitespace? c) (memv c demo/slug-separators)))

(define (demo/slug title)
  (let loop ([chars (string->list title)] [dash #f] [out '()])
    (cond
      [(null? chars) (string-downcase (list->string (reverse out)))]
      [(demo/separator? (car chars)) (loop (cdr chars) (pair? out) out)]
      [else (loop (cdr chars) #f (cons (car chars) (if dash (cons #\- out) out)))])))

(define (demo/headings lines)
  (let loop ([lines lines] [line 0] [found '()])
    (cond
      [(null? lines) (reverse found)]
      [(starts-with? (car lines) "## ")
       (loop (cdr lines)
             (+ line 1)
             (cons (cons line (substring (car lines) 3 (string-length (car lines)))) found))]
      [else (loop (cdr lines) (+ line 1) found)])))

(define (demo/entry heading)
  (string-append "- [" (cdr heading) "](#" (demo/slug (cdr heading)) ")\n"))

(define-typed-command! "demo-toc"
  "Insert a table of contents linking every `##` heading, above the first one."
  (lambda (pane)
    (let ([headings (demo/headings (buffer-lines pane))])
      (if (null? headings)
          (error "demo-toc: the buffer has no ## heading")
          (let ([at (line->char-offset pane (car (car headings)))]
                [toc (string-append (apply string-append (map demo/entry headings)) "\n")])
            (edit-text! pane (list (hash 'start-char at 'end-char at 'text toc))))))))
