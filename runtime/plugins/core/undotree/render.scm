;;; core:undotree — render.scm. See README.md's "Graph".

(provide undotree/format-age undotree/render undotree/row-index)

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

(define (undotree/remember-layout! key graphs width)
  (let ([entry (cons graphs width)])
    (set-box! undotree/*layout-memo* (cons key entry))
    entry))

(define (undotree/appends-child? key previous-key)
  (and (equal? (cdr (car key)) (car (car previous-key)))
       (equal? (cdr key) previous-key)))

(define (undotree/graphs newest-first)
  (let ([key (map (lambda (node) (cons (hash-ref node 'id) (hash-ref node 'parent)))
                  newest-first)]
        [memo (unbox undotree/*layout-memo*)])
    (cond
      [(and memo (equal? (car memo) key)) (cdr memo)]
      [(and memo (undotree/appends-child? key (car memo)))
       (let ([previous (cdr memo)])
         (undotree/remember-layout! key
                                    (vector-append (vector "o") (car previous))
                                    (cdr previous)))]
      [else
       (let ([graphs (undotree/layout newest-first '() '())])
         (undotree/remember-layout! key (list->vector graphs) (undotree/max-width graphs)))])))

;; ── Rows ─────────────────────────────────────────────────────────────────────

(define (undotree/spaces s width)
  (make-string (max 0 (- width (string-length s))) #\space))

(define (undotree/pad-right s width)
  (string-append s (undotree/spaces s width)))

(define (undotree/pad-left s width)
  (string-append (undotree/spaces s width) s))

(define (undotree/max-width strings)
  (apply max (map string-length strings)))

(define (undotree/format-row node graph age graph-width age-width)
  (string-append
   (undotree/pad-right graph graph-width)
   "  "
   (if (hash-ref node 'current?) "@" " ")
   (if (hash-ref node 'saved?) "S" " ")
   " "
   (undotree/pad-left age age-width)))

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
  (let* ([newest-first (reverse nodes)]
         [layout (undotree/graphs newest-first)]
         [graphs (car layout)]
         [graph-width (cdr layout)]
         [age-width (max 3 (string-length
                            (undotree/format-age (hash-ref (car nodes) 'age-secs))))]
         [current (undotree/find-row (lambda (node) (hash-ref node 'current?)) newest-first 0)])
    (hash 'nodes newest-first
          'render (lambda (start keys)
                    (let loop ([keys keys] [i start] [acc '()])
                      (if (null? keys)
                          (reverse acc)
                          (loop (cdr keys) (+ i 1)
                                (cons (undotree/format-row
                                       (car keys) (vector-ref graphs i)
                                       (undotree/format-age (hash-ref (car keys) 'age-secs))
                                       graph-width age-width)
                                      acc)))))
          'current (car current)
          'current-node (cdr current)
          'next-change-secs (apply min (map (lambda (node)
                                              (undotree/secs-to-label-change
                                               (hash-ref node 'age-secs)))
                                            nodes)))))
