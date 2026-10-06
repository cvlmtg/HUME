;;; core:lsp/locations.scm — the locations drawer. See docs/architecture.md.

(require "lib.scm")

(provide lsp/show-locations! lsp/answer-locations)

;; A goto or references result as a list of locations, '() for none.
(define (lsp/response-locations res)
  (cond
    ((void? res) '())
    ((json-array? res) (json-list res))
    (else (list res))))

;; Every location in an `lsp-request-all!` answer, in server order, each
;; server's error reported.
(define (lsp/answer-locations what results)
  (lsp/report-answer-errors! what results)
  (apply append (map lsp/response-locations (lsp/answers results))))

(define (lsp/location-display part)
  (let* ((path (path->display (hash-ref part 'path)))
         (line (hash-ref part 'line))
         (grapheme-col-or-wire (hash-ref part 'grapheme-col-or-wire)))
    (string-append path ":"
      (if grapheme-col-or-wire
          (lsp/format-position line grapheme-col-or-wire)
          (number->string (+ 1 line))))))

;; The open list's session, or #f: one at a time, since the drawer is one slot.
(define lsp/*locations* #f)

(define (lsp/end-locations-session!)
  (when lsp/*locations*
    (untrack-position! (hash-ref lsp/*locations* 'tracked))
    (set! lsp/*locations* #f)))

(define (lsp/close-locations! message)
  (when lsp/*locations*
    (close-drawer! (hash-ref lsp/*locations* 'drawer))
    (lsp/end-locations-session!)
    (log! 'info message)))

(define (lsp/locations-on-select parts)
  (lambda (idx drawer)
    (cond
      (idx (goto-location! (focused-pane) (hash-ref (list-ref parts idx) 'location)))
      ((and lsp/*locations* (equal? (hash-ref lsp/*locations* 'drawer) drawer))
       (lsp/end-locations-session!)))))

(define (lsp/location-line-counts pane parts)
  (let loop ((parts parts)
             (counts (hash (buffer-key pane) (buffer-line-count pane))))
    (if (null? parts)
        counts
        (let ((buffer (hash-ref (car parts) 'buffer)))
          (loop (cdr parts)
                (if buffer
                    (hash-insert counts (buffer-key buffer) (buffer-line-count buffer))
                    counts))))))

;; `parts` are the rows to list; `tracked` is the request's position; `shape`
;; maps position params to request params.
(define (lsp/show-locations! pane tracked parts method shape not-found)
  (lsp/end-locations-session!)
  (let ((drawer (show-drawer-list! (focused-pane) (map lsp/location-display parts)
                                   (lsp/locations-on-select parts))))
    (when drawer
      (let ((counts (lsp/location-line-counts pane parts)))
        (keep-tracked-position! tracked)
        (set! lsp/*locations*
              (hash 'drawer drawer 'tracked tracked 'pane pane
                    'method method 'shape shape 'not-found not-found 'seq 0
                    'counts counts))))))

;; ── Refresh ──

(define (lsp/swap-locations! session parts)
  (let* ((drawer (hash-ref session 'drawer))
         (selected (drawer-selected-index drawer)))
    (if (not selected)
        (lsp/end-locations-session!)
        (let ((idx (min selected (- (length parts) 1))))
          (if (update-drawer-list! drawer (map lsp/location-display parts)
                                   (lsp/locations-on-select parts) idx)
              (set! lsp/*locations*
                    (hash-insert session 'counts
                                 (lsp/location-line-counts (hash-ref session 'pane) parts)))
              (lsp/end-locations-session!))))))

(define (lsp/apply-locations! drawer seq err results)
  (let ((session lsp/*locations*))
    (when (and session (equal? (hash-ref session 'drawer) drawer) (= (hash-ref session 'seq) seq))
      (cond
        (err (lsp/report-error! "locations" err))
        ;; No server answered: the rows shown are still the best known.
        ((null? (filter (lambda (r) (not (hash-ref r 'err))) results))
         (lsp/report-answer-errors! "locations" results))
        (else
         (let ((parts (lsp-locations->display-parts (lsp/answer-locations "locations" results))))
           (if (null? parts)
               (lsp/close-locations! (hash-ref session 'not-found))
               (lsp/swap-locations! session parts))))))))

(define (lsp/refresh-locations!)
  (when lsp/*locations*
    (let* ((session lsp/*locations*)
           (params (and (drawer-selected-index (hash-ref session 'drawer))
                        (tracked-position-params (hash-ref session 'tracked)))))
      (if (not params)
          (lsp/end-locations-session!)
          (let ((seq (+ 1 (hash-ref session 'seq)))
                (drawer (hash-ref session 'drawer)))
            (set! lsp/*locations* (hash-insert session 'seq seq))
            (lsp-request-all! (hash-ref session 'pane) (hash-ref session 'method)
                          ((hash-ref session 'shape) params)
                          (lambda (err results) (lsp/apply-locations! drawer seq err results))
                          #:allow-stale #t #:supersede "lsp-locations"))))))

(define lsp/refresh-locations (debounce 300 lsp/refresh-locations!))

(define (lsp/locations-text-changed pane)
  (when lsp/*locations*
    (let ((counts (hash-ref lsp/*locations* 'counts))
          (key (buffer-key pane)))
      (when (hash-contains? counts key)
        (let ((now (buffer-line-count pane)))
          (unless (= now (hash-ref counts key))
            (set! lsp/*locations*
                  (hash-insert lsp/*locations* 'counts (hash-insert counts key now)))
            (lsp/refresh-locations)))))))

(register-hook! 'on-text-changed lsp/locations-text-changed)
