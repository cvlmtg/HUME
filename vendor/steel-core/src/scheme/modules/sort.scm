(provide sort)

(define (list->mutable-vector lst)
  (let ([v (mutable-vector)])
    (for-each (lambda (i) (vector-push! v i)) lst)
    v))

(define (sort l less?)
  (if (#%function-pointer? less?)
      (#%list-sort l less?)
      (mutable-vector->list
        (merge-sort (list->mutable-vector l) (list->mutable-vector l) less?))))

;; Bottom-up merge sort: merges runs of `width` from `src` into `dst`,
;; doubling `width` and swapping the two each pass, and returns the vector
;; holding the result. O(n log n) on any input, and stable.
(define (merge-sort src dst less?)
  (define n (vector-length src))
  (let pass ([src src] [dst dst] [width 1])
    (if (>= width n)
        src
        (begin
          (merge-runs! src dst 0 width n less?)
          (pass dst src (* width 2))))))

(define (merge-runs! src dst left width n less?)
  (if (< left n)
      (let* ([mid (if (< (+ left width) n) (+ left width) n)]
             [right (if (< (+ mid width) n) (+ mid width) n)])
        (merge! src dst left mid right less?)
        (merge-runs! src dst right width n less?))
      dst))

;; An element of the right run goes first only when it is strictly less.
(define (merge! src dst left mid right less?)
  (let loop ([i left] [j mid] [k left])
    (if (< k right)
        (if (and (< i mid)
                 (or (>= j right) (not (less? (vector-ref src j) (vector-ref src i)))))
            (begin
              (vector-set! dst k (vector-ref src i))
              (loop (+ i 1) j (+ k 1)))
            (begin
              (vector-set! dst k (vector-ref src j))
              (loop i (+ j 1) (+ k 1))))
        dst)))

;;; Hansen's original code for this benchmark used Larceny's
;;; predefined random procedure.  When Marc Feeley modified
;;; Hansen's benchmark for the Gambit benchmark suite, however,
;;; he added a specific random number generator taken from an
;;; article in CACM.  Feeley's generator used bignums, and was
;;; extremely slow, causing the Gambit version of this benchmark
;;; to spend nearly all of its time generating the random numbers.
;;; For a benchmark called quicksort to become a bignum benchmark
;;; was very misleading, so Clinger left Feeley's version of this
;;; benchmark out of the Larceny benchmark suite.
;;;
;;; The following random number generator is much better and
;;; faster than the one used in the Gambit benchmark.  See
;;;
;;; http://srfi.schemers.org/srfi-27/mail-archive/msg00000.html
;;; http://www.math.purdue.edu/~lucier/random/random.scm

;;; A uniform [0,1] random number generator; is
;;; Pierre L'Ecuyer's generator from his paper
;;; "Good parameters and implementations for combined multiple
;;; recursive random number generators"
;;; available at his web site http://www.iro.umontreal.ca/~lecuyer

(define seed-set! #f)
(define seed-ref #f)
(define random-flonum #f)

(let ([norm 2.328306549295728e-10]
      [m1 4294967087.0]
      [m2 4294944443.0]
      [a12 1403580.0]
      [a13n 810728.0]
      [a21 527612.0]
      [a23n 1370589.0]
      [seed (vector 1.0 0.0 0.0 1.0 0.0 0.0)]) ;; will be mutated

  ;; uses no conversions between flonums and fixnums.

  (set! random-flonum
        (lambda ()
          (let ([seed seed]) ;; make it local
            (let ([p1 (- (* a12 (vector-ref seed 1)) (* a13n (vector-ref seed 0)))]
                  [p2 (- (* a21 (vector-ref seed 5)) (* a23n (vector-ref seed 3)))])
              (let ([k1 (truncate (/ p1 m1))]
                    [k2 (truncate (/ p2 m2))]
                    [ignore1 (vector-set! seed 0 (vector-ref seed 1))]
                    [ignore3 (vector-set! seed 3 (vector-ref seed 4))])
                (let ([p1 (- p1 (* k1 m1))]
                      [p2 (- p2 (* k2 m2))]
                      [ignore2 (vector-set! seed 1 (vector-ref seed 2))]
                      [ignore4 (vector-set! seed 4 (vector-ref seed 5))])
                  (let ([p1 (if (< p1 0.0)
                                (+ p1 m1)
                                p1)]
                        [p2 (if (< p2 0.0)
                                (+ p2 m2)
                                p2)])
                    (vector-set! seed 2 p1)
                    (vector-set! seed 5 p2)
                    (if (<= p1 p2)
                        (* norm (+ (- p1 p2) m1))
                        (* norm (- p1 p2))))))))))

  (set! seed-ref (lambda () (vector->list seed)))

  (set! seed-set! (lambda l (set! seed (list->vector l)))))

(define (random n)
  (exact (truncate (* (inexact n) (random-flonum)))))
