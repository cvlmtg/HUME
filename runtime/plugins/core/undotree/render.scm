;;; core:undotree — render.scm. See README.md's "Graph".

(provide undotree/format-age undotree/render undotree/row-index)

;;; See README.md's "Age".
(define (undotree/format-age secs)
  (cond
    [(< secs 60) (string-append (number->string secs) "s")]
    [(< secs 3600) (string-append (number->string (quotient secs 60)) "m")]
    [(< secs 86400) (string-append (number->string (quotient secs 3600)) "h")]
    [else (string-append (number->string (quotient secs 86400)) "d")]))

;; ── Lanes ────────────────────────────────────────────────────────────────────

(define (undotree/lanes-waiting-for lanes id)
  (filter (lambda (i) (equal? (list-ref lanes i) id))
          (range 0 (length lanes))))

(define (undotree/first-free-lane lanes)
  (let ([free (undotree/lanes-waiting-for lanes #f)])
    (if (null? free) (length lanes) (car free))))

(define (undotree/trim-reversed rev)
  (if (and (pair? rev) (not (car rev)))
      (undotree/trim-reversed (cdr rev))
      (reverse rev)))

(define (undotree/advance-lanes lanes column merging parent)
  (undotree/trim-reversed
   (reverse
    (map (lambda (i)
           (cond
             [(= i column) parent]
             [(member i merging) #f]
             [else (list-ref lanes i)]))
         (range 0 (length lanes))))))

;; ── Graph cells ──────────────────────────────────────────────────────────────

(define (undotree/graph-cell lanes i column merging)
  (cond
    [(= i column) "o"]
    [(member i merging) "'"]
    [(list-ref lanes i) "|"]
    [else " "]))

(define (undotree/graph-gap i column merging)
  (if (and (pair? merging) (<= column i) (< i (apply max merging))) "-" " "))

(define (undotree/graph-row lanes column merging)
  (let ([end (- (length lanes) 1)])
    (apply string-append
           (map (lambda (i)
                  (string-append (undotree/graph-cell lanes i column merging)
                                 (if (< i end) (undotree/graph-gap i column merging) "")))
                (range 0 (length lanes))))))

;;; See README.md's "Graph".
(define (undotree/layout nodes lanes laid-out)
  (if (null? nodes)
      (reverse laid-out)
      (let* ([node (car nodes)]
             [waiting (undotree/lanes-waiting-for lanes (hash-ref node 'id))]
             [column (if (null? waiting) (undotree/first-free-lane lanes) (car waiting))]
             [merging (if (null? waiting) '() (cdr waiting))]
             [wide (if (= column (length lanes)) (append lanes (list #f)) lanes)])
        (undotree/layout
         (cdr nodes)
         (undotree/advance-lanes wide column merging (hash-ref node 'parent))
         (cons (hash 'node node
                     'graph (undotree/graph-row wide column merging)
                     'age (undotree/format-age (hash-ref node 'age-secs)))
               laid-out)))))

;; ── Rows ─────────────────────────────────────────────────────────────────────

(define (undotree/spaces s width)
  (make-string (max 0 (- width (string-length s))) #\space))

(define (undotree/pad-right s width)
  (string-append s (undotree/spaces s width)))

(define (undotree/pad-left s width)
  (string-append (undotree/spaces s width) s))

(define (undotree/max-width strings)
  (apply max (map string-length strings)))

(define (undotree/format-row row graph-width age-width)
  (let ([node (hash-ref row 'node)])
    (string-append
     (undotree/pad-right (hash-ref row 'graph) graph-width)
     "  "
     (if (hash-ref node 'current?) "@" " ")
     (if (hash-ref node 'saved?) "S" " ")
     " "
     (undotree/pad-left (hash-ref row 'age) age-width))))

(define (undotree/find-row pred rows i)
  (cond
    [(null? rows) #f]
    [(pred (car rows)) (cons i (car rows))]
    [else (undotree/find-row pred (cdr rows) (+ i 1))]))

(define (undotree/row-index pred rows)
  (let ([hit (undotree/find-row pred rows 0)])
    (and hit (car hit))))

;;; See README.md's "Graph" for the input and result shapes.
(define (undotree/render nodes)
  (let* ([laid-out (undotree/layout (reverse nodes) '() '())]
         [node-of (lambda (row) (hash-ref row 'node))]
         [ids (map (lambda (row) (hash-ref (node-of row) 'id)) laid-out)]
         [graph-width (undotree/max-width (map (lambda (row) (hash-ref row 'graph)) laid-out))]
         [age-width (undotree/max-width (map (lambda (row) (hash-ref row 'age)) laid-out))]
         [current (undotree/find-row (lambda (row) (hash-ref (node-of row) 'current?)) laid-out 0)])
    (hash 'rows (map (lambda (row) (undotree/format-row row graph-width age-width))
                     laid-out)
          'ids ids
          'current (car current)
          'current-node (node-of (cdr current)))))
