;;; core:lsp/locations.scm — the locations drawer. See docs/architecture.md.

(require "helpers.scm")

(provide lsp/show-locations! lsp/answer-locations lsp/place-display
         lsp/report-not-found! lsp/report-skipped!)

;; A goto or references result as a list of locations, '() for none.
(define (lsp/response-locations res)
  (cond
    ((void? res) '())
    ((json-array? res) (json-list res))
    (else (list res))))

;; Logs `message` when the answer held no locations. Locations with no place
;; to jump to are reported by `lsp/report-skipped!`.
(define (lsp/report-not-found! locations message)
  (when (null? locations) (log! 'info message)))

;; Warns once when `count` locations of an answer name no place to jump to.
(define (lsp/report-skipped! what count)
  (when (> count 0)
    (log! 'warning (string-append "lsp " what ": skipped " (number->string count)
                               (if (= count 1) " location" " locations")
                               " HUME cannot jump to"))))

;; Every location in an `lsp-request-all!` answer, in server order, each
;; server's error reported.
(define (lsp/answer-locations what results)
  (lsp/report-answer-errors! what results)
  (apply append (map lsp/response-locations (lsp/answers results))))

(define (lsp/place-display place)
  (string-append (path->display (hash-ref place 'path)) ":"
                 (lsp/format-position (hash-ref place 'line)
                                      (hash-ref place 'grapheme-col-or-wire))))

;; The open list's session, or #f: one at a time, since the drawer is one slot.
(define lsp/*locations* #f)

(define (lsp/end-locations-session!)
  (when lsp/*locations*
    (untrack-position! (hash-ref lsp/*locations* 'tracked))
    (set! lsp/*locations* #f)))

(define (lsp/close-locations!)
  (when lsp/*locations*
    (close-drawer! (hash-ref lsp/*locations* 'drawer))
    (lsp/end-locations-session!)))

(define (lsp/locations-on-select place drawer)
  (cond
    (place (lsp/jump-to! (hash-ref place 'location)))
    ((and lsp/*locations* (equal? (hash-ref lsp/*locations* 'drawer) drawer))
     (lsp/end-locations-session!))))

(define (lsp/place-line-counts pane places)
  (let loop ((places places)
             (counts (hash (buffer-key pane) (buffer-line-count pane))))
    (if (null? places)
        counts
        (let ((buffer (hash-ref (car places) 'buffer)))
          (loop (cdr places)
                (if buffer
                    (hash-insert counts (buffer-key buffer) (buffer-line-count buffer))
                    counts))))))

;; `places` are the rows to list; `tracked` is the request's position; `shape`
;; maps position params to request params.
(define (lsp/show-locations! pane tracked places method shape not-found)
  (lsp/end-locations-session!)
  (let ((drawer (show-drawer-list! (focused-pane) (lsp/payload-rows lsp/place-display places)
                                   lsp/locations-on-select)))
    (when drawer
      (let ((counts (lsp/place-line-counts pane places)))
        (keep-tracked-position! tracked)
        (set! lsp/*locations*
              (hash 'drawer drawer 'tracked tracked 'pane pane
                    'method method 'shape shape 'not-found not-found 'seq 0
                    'counts counts))))))

;; ── Refresh ──

(define (lsp/swap-locations! session places)
  (let* ((drawer (hash-ref session 'drawer))
         (selected (drawer-selected-index drawer)))
    (if (not selected)
        (lsp/end-locations-session!)
        (let ((idx (min selected (- (length places) 1))))
          (if (update-drawer-list! drawer (lsp/payload-rows lsp/place-display places)
                                   lsp/locations-on-select idx)
              (set! lsp/*locations*
                    (hash-insert session 'counts
                                 (lsp/place-line-counts (hash-ref session 'pane) places)))
              (lsp/end-locations-session!))))))

(define (lsp/apply-locations! drawer seq err results)
  (let ((session lsp/*locations*))
    (when (and session (equal? (hash-ref session 'drawer) drawer) (= (hash-ref session 'seq) seq))
      (cond
        (err (lsp/report-error! "locations" err))
        ;; No server answered: the rows shown are still the best known.
        ((lsp/none-answered? results)
         (lsp/report-answer-errors! "locations" results))
        (else
         (let* ((locations (lsp/answer-locations "locations" results))
                (merged (lsp-locations->merged-places locations))
                (places (hash-ref merged 'places)))
           (if (null? places)
               (begin
                 (lsp/close-locations!)
                 (lsp/report-skipped! "locations" (hash-ref merged 'skipped))
                 (lsp/report-not-found! locations (hash-ref session 'not-found)))
               (lsp/swap-locations! session places))))))))

(define lsp/locations-key (lsp-supersede-key))

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
                          #:allow-stale #t #:supersede lsp/locations-key))))))

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
