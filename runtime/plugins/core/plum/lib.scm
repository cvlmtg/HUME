;;; core:plum/lib.scm

(provide plum/valid-dir-entry? plum/batch-run plum/find)

;; ── Directory entry filter ────────────────────────────────────────────────────

;;; Return #t if `name` is a valid, traversable directory entry (not "." or "..").
(define (plum/valid-dir-entry? name)
  (and (not (equal? name "."))
       (not (equal? name ".."))))

;; ── List search ───────────────────────────────────────────────────────────────

;;; First element of `lst` satisfying `pred?`, or `#f`.
(define (plum/find pred? lst)
  (cond ((null? lst) #f)
        ((pred? (car lst)) (car lst))
        (else (plum/find pred? (cdr lst)))))

;; ── Batch runner ──────────────────────────────────────────────────────────────

;;; Run `thunk` on each name in `names`, collecting errors rather than
;;; aborting.  Logs per-item progress and a summary at the end.
;;; Returns the count of successful thunk calls.
(define (plum/batch-run verb names thunk)
  (let loop ((names names) (ok 0) (errs '()))
    (cond
      ((null? names)
       (log! 'info
             (string-append "PLUM: "
                            (number->string ok) " " verb
                            " — "
                            (number->string (length errs)) " failed"))
       (for-each (lambda (e) (log! 'error e)) (reverse errs))
       ok)
      (else
       (let ((name (car names)))
         (log! 'info (string-append "PLUM: " verb " " name))
         (with-handler
           (lambda (err)
             (loop (cdr names) ok
                   (cons (string-append "  " name ": " (to-string err)) errs)))
           (begin
             (thunk name)
             (loop (cdr names) (+ ok 1) errs))))))))
