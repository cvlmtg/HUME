;;; core:lsp/lib.scm — shared helpers used by every feature file.

(provide lsp/supports? lsp/guard-capability lsp/report-error lsp/visible-lines)

;; ── Capability guard ────────────────────────────────────────────────────────

;;; #t if the focused buffer's attached server advertises `cap-key` (e.g.
;;; "hoverProvider") — missing or explicitly `#f` means unsupported.
(define (lsp/supports? cap-key)
  (let ((caps (lsp-capabilities #f)))
    (and caps
         (hash-contains? caps cap-key)
         (not (equal? (hash-ref caps cap-key) #f)))))

;;; Run `thunk` only if the focused buffer's server supports `cap-key`;
;;; otherwise report politely and skip — every feature capability-checks
;;; before firing a request (hub primer).
(define (lsp/guard-capability cap-key thunk)
  (if (lsp/supports? cap-key)
      (thunk)
      (log! 'info
            (string-append "not supported by "
                           (let ((name (lsp-server-for-buffer (current-buffer))))
                             (if name name "server"))))))

;;; One `'error` log line for a B2 callback error — `err` is either a
;;; {"code" "message"} hashmap (protocol error) or a bare string
;;; ("timeout"/"server-crashed").
(define (lsp/report-error what err)
  (log! 'error
        (string-append "lsp " what ": "
                       (if (string? err) err (hash-ref err "message")))))

;; ── Viewport tracker ────────────────────────────────────────────────────────
;; No pane-geometry builtin exists — on-viewport-change (B7) is the only
;; Steel-visible viewport source. Buffer-id SteelVals are NOT `equal?` across
;; separate wrappings of the same underlying id (Arc-pointer equality), so
;; per-buffer state is an assoc-list searched with `buffer-id=?`, never a
;; hashmap keyed directly by a bid.

(define *lsp-viewports* '())  ; list of (bid first last)

(register-hook! 'on-viewport-change
  (lambda (bid first last)
    (set! *lsp-viewports*
          (cons (list bid first last)
                (filter (lambda (e) (not (buffer-id=? (list-ref e 0) bid)))
                        *lsp-viewports*)))))

;;; Number of lines visible in `bid`'s pane as of the last on-viewport-change
;;; fire, or `#f` before the first event for that buffer.
(define (lsp/visible-lines bid)
  (let ((matches (filter (lambda (e) (buffer-id=? (list-ref e 0) bid)) *lsp-viewports*)))
    (if (null? matches)
        #f
        (let ((entry (car matches)))
          (+ 1 (- (list-ref entry 2) (list-ref entry 1)))))))
