;;; core:markdown — headings.scm. See README.md's Commands.

(require "queries.scm")
(require "lists.scm")
(require "render.scm")

(provide md-headings/set!)

;;; The ATX heading `text` holds, as `(level marker-start marker-end)`: the
;;; marker's char columns, `marker-end` past the blanks after it. #f off one.
(define (md-headings/atx text)
  (let* ([indent (md-preview/skip-blanks text 0)]
         [hashes (md-preview/skip-while text indent (lambda (c) (char=? c #\#)))]
         [level (- hashes indent)])
    (and (>= level 1)
         (<= level 6)
         (or (= hashes (string-length text))
             (md-preview/blank? (string-ref text hashes)))
         (list level indent (md-preview/skip-blanks text hashes)))))

;;; The edit that makes `info`'s line a level-`level` heading, or #f. A level
;;; of 0 toggles: a heading loses its marker, and a line with text gains one.
(define (md-headings/edit info level)
  (let* ([start (car info)]
         [text (cdr info)]
         [heading (md-headings/atx text)]
         [indent (md-preview/skip-blanks text 0)]
         [target (cond [(> level 0) level] [heading 0] [else 1])])
    (cond
      [(and heading (= target 0))
       (md-preview/edit (+ start (cadr heading)) (+ start (caddr heading)) "")]
      [(and heading (> target 0))
       (md-preview/edit (+ start (cadr heading)) (+ start (caddr heading))
                        (string-append (make-string target #\#) " "))]
      [(and (> target 0) (not (md-preview/blank-from? text 0)))
       (md-preview/insert (+ start indent) (string-append (make-string target #\#) " "))]
      [else #f])))

;;; Every line a selection touches gets a level-1 heading or loses one, or a
;;; level-`level` heading when `level` is above 0.
(define (md-headings/set! pane level)
  (let* ([reader (md-preview/line-reader pane)]
         [line-of (md-preview/lines-line-of reader)]
         [line-info (md-preview/lines-info reader)]
         [spans (map (lambda (sel)
                       (md-preview/lines-of line-of (selection-start-char sel) (selection-end-char sel)))
                     (buffer-selections pane))]
         [lines (md-lists/distinct
                  (call! "stdlib/concat"
                         (map (lambda (span) (range (car span) (cdr span))) spans)))]
         [edits (filter (lambda (edit) edit)
                        (map (lambda (line) (md-headings/edit (line-info line) level)) lines))])
    (unless (null? edits)
      (edit-text! pane edits))))
