;;; core:undotree — render.scm. See README.md's "Graph".

(provide undotree/render)

;;; See README.md's "Age".
(define (undotree/age-unit secs)
  (cond
    [(< secs 60) (cons 1 "s")]
    [(< secs 3600) (cons 60 "m")]
    [(< secs 86400) (cons 3600 "h")]
    [else (cons 86400 "d")]))

(define (undotree/format-age secs)
  (let ([unit (undotree/age-unit secs)])
    (string-append (number->string (quotient secs (car unit))) (cdr unit))))

(define (undotree/secs-to-label-change secs)
  (let ([divisor (car (undotree/age-unit secs))])
    (- divisor (remainder secs divisor))))

;; ── Lanes ────────────────────────────────────────────────────────────────────

(define (undotree/lanes-waiting-for lanes id)
  (let loop ([lanes lanes] [i 0] [found '()])
    (cond
      [(null? lanes) (reverse found)]
      [(equal? (car lanes) id) (loop (cdr lanes) (+ i 1) (cons i found))]
      [else (loop (cdr lanes) (+ i 1) found)])))

(define (undotree/first-free-lane lanes)
  (let ([free (undotree/lanes-waiting-for lanes #f)])
    (if (null? free) (length lanes) (car free))))

(define (undotree/trim-reversed rev)
  (if (and (pair? rev) (not (car rev)))
      (undotree/trim-reversed (cdr rev))
      (reverse rev)))

(define (undotree/advance-lanes lanes column merging parent)
  (let loop ([lanes lanes] [i 0] [rev '()])
    (if (null? lanes)
        (undotree/trim-reversed rev)
        (loop (cdr lanes) (+ i 1)
              (cons (cond
                      [(= i column) parent]
                      [(member i merging) #f]
                      [else (car lanes)])
                    rev)))))

;; ── Graph cells ──────────────────────────────────────────────────────────────

(define (undotree/graph-cell lane i column merging)
  (cond
    [(= i column) "o"]
    [(member i merging) "'"]
    [lane "|"]
    [else " "]))

(define (undotree/graph-row lanes column merging)
  (let ([bus-end (if (pair? merging) (apply max merging) column)])
    (let loop ([lanes lanes] [i 0] [rev '()])
      (if (null? lanes)
          (apply string-append (reverse rev))
          (let ([cell (undotree/graph-cell (car lanes) i column merging)])
            (loop (cdr lanes) (+ i 1)
                  (cond
                    [(null? (cdr lanes)) (cons cell rev)]
                    [(and (<= column i) (< i bus-end)) (cons "-" (cons cell rev))]
                    [else (cons " " (cons cell rev))])))))))

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
         (cons (undotree/graph-row wide column merging) laid-out)))))

;;; See README.md's "Graph".
(define undotree/*layout-memo* (box #f))

(define (undotree/appends-child? key previous-key)
  (and (equal? (cdr (car key)) (car (car previous-key)))
       (equal? (cdr key) previous-key)))

(define (undotree/positions oldest-first)
  (let loop ([nodes oldest-first] [i 0] [acc (hash)])
    (if (null? nodes)
        acc
        (loop (cdr nodes) (+ i 1) (hash-insert acc (hash-ref (car nodes) 'id) i)))))

(define (undotree/graphs oldest-first newest-first)
  (let* ([key (map (lambda (node) (cons (hash-ref node 'id) (hash-ref node 'parent)))
                   newest-first)]
         [memo (unbox undotree/*layout-memo*)]
         [layout
          (cond
            [(and memo (equal? (car memo) key)) (cdr memo)]
            [(and memo (undotree/appends-child? key (car memo)))
             (let* ([previous (cdr memo)]
                    [graphs (hash-ref previous 'graphs)])
               (hash 'graphs (vector-append graphs (vector "o"))
                     'width (hash-ref previous 'width)
                     'position (hash-insert (hash-ref previous 'position)
                                            (car (car key)) (vector-length graphs))))]
            [else
             (let ([graphs (undotree/layout newest-first '() '())])
               (hash 'graphs (list->vector (reverse graphs))
                     'width (apply max (map string-length graphs))
                     'position (undotree/positions oldest-first)))])])
    (set-box! undotree/*layout-memo* (cons key layout))
    layout))

;; ── Rows ─────────────────────────────────────────────────────────────────────

(define (undotree/spaces s width)
  (make-string (- width (string-length s)) #\space))

(define (undotree/pad-right s width)
  (string-append s (undotree/spaces s width)))

(define (undotree/pad-left s width)
  (string-append (undotree/spaces s width) s))

(define (undotree/span start end scope)
  (hash 'start-char start 'end-char end 'scope scope))

(define undotree/current-scope "diagnostic.warning.message")
(define undotree/saved-scope "diagnostic.info.message")

(define (undotree/marker-span flag cell scope selected-scope)
  (if flag
      (list (hash-insert (undotree/span cell (+ cell 1) scope)
                         'selected-scope selected-scope))
      '()))

;;; See README.md's "Graph" for the scopes.
(define (undotree/format-row node graph age graph-width age-width)
  (let* ([current? (hash-ref node 'current?)]
         [saved? (hash-ref node 'saved?)]
         [row (hash 'text (string-append
                           " "
                           (undotree/pad-right graph graph-width)
                           "  "
                           (if current? "@" " ")
                           (if saved? "S" " ")
                           " "
                           (undotree/pad-left age age-width))
                    'spans (append
                            (list (undotree/span 1 (+ 1 (string-length graph)) "markup.bold"))
                            (undotree/marker-span current? (+ graph-width 3)
                                                  "diagnostic.warning.message-text"
                                                  undotree/current-scope)
                            (undotree/marker-span saved? (+ graph-width 4)
                                                  "diagnostic.info.message-text"
                                                  undotree/saved-scope)))])
    (cond
      [current? (hash-insert row 'selected-scope undotree/current-scope)]
      [saved? (hash-insert row 'selected-scope undotree/saved-scope)]
      [else row])))

;;; See README.md's "Graph" for the input and result shapes.
(define (undotree/render nodes)
  (let* ([newest-first (reverse nodes)]
         [layout (undotree/graphs nodes newest-first)]
         [graphs (hash-ref layout 'graphs)]
         [position (hash-ref layout 'position)]
         [graph-width (hash-ref layout 'width)]
         [newest (- (vector-length graphs) 1)]
         [row-of (lambda (id)
                   (let ([i (hash-try-get position id)])
                     (and i (- newest i))))]
         [age-width (max 3 (string-length
                            (undotree/format-age (hash-ref (car nodes) 'age-secs))))]
         [current-node (findf (lambda (node) (hash-ref node 'current?)) newest-first)])
    (hash 'nodes newest-first
          'render (lambda (rows budget)
                    (map (lambda (node)
                           (undotree/format-row
                            node (vector-ref graphs (hash-ref position (hash-ref node 'id)))
                            (undotree/format-age (hash-ref node 'age-secs))
                            graph-width age-width))
                         rows))
          'row-of row-of
          'current (row-of (hash-ref current-node 'id))
          'current-node current-node
          'next-change-secs (apply min (map (lambda (node)
                                              (undotree/secs-to-label-change
                                               (hash-ref node 'age-secs)))
                                            nodes)))))
