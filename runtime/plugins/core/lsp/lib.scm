;;; core:lsp/lib.scm — shared helpers used by every feature file. See
;;; docs/architecture.md.

(provide lsp/report-error! lsp/answered lsp/answers lsp/none-answered? lsp/report-answer-errors! lsp/with-position-params
         lsp/visible-lines lsp/resolve-pane
         lsp/setup-trigger-chars! lsp/format-position lsp/cap-field lsp/cap-flag?
         lsp/with-servers)

;; ── Capabilities ────────────────────────────────────────────────────────────

;; `cap` is a server's `(lsp-capability …)`: `#t`, an options object, or `#f`.
(define (lsp/cap-field cap field default)
  (if (json-object? cap) (json-ref-or cap default field) default))

(define (lsp/cap-flag? cap field)
  (equal? (lsp/cap-field cap field #f) #t))

;; Calls `(proceed params)` with `pane`'s position params, or logs that the
;; buffer has no file when it has none.
(define (lsp/with-position-params pane what proceed)
  (let ((params (lsp-position-params pane)))
    (if params
        (proceed params)
        (log! 'info (string-append "lsp " what ": this buffer has no file")))))

;; Calls `(proceed servers)` with `servers`, or logs that `what` is unsupported when empty.
(define (lsp/with-servers servers what proceed)
  (if (null? servers)
      (log! 'info (string-append what " is not supported by this buffer's language servers"))
      (proceed servers)))

;; ── Trigger-char lifecycle ──────────────────────────────────────────────────

(define (lsp/setup-trigger-chars! feature source-name extra-chars on-trigger)
  (define (set-chars! pane server chars)
    (if on-trigger
        (set-attachment-hook-triggers! source-name pane server feature chars)
        (set-attachment-completion-triggers! source-name pane server feature chars)))
  (register-hook! 'on-lsp-attach
    (lambda (pane server)
      (let ((tc (lsp/cap-field (lsp-capability server #:feature feature) "triggerCharacters" #f)))
        (set-chars! pane server (append extra-chars (if tc (json-list tc) (list)))))))
  (when on-trigger
    (register-hook! 'on-trigger-char
      (lambda (pane ch source)
        (when (equal? source source-name)
          (on-trigger pane ch))))))

;; See docs/architecture.md, "Response conventions".
(define (lsp/report-error! what err)
  (log! (if (member (hash-ref err 'kind) '(unavailable stopped)) 'info 'error)
        (string-append "lsp " what ": " (hash-ref err 'message))))

;; The entries of an `lsp-request-all!` answer that are neither an error nor
;; null, in server order.
(define (lsp/answered results)
  (filter (lambda (r) (not (or (hash-ref r 'err) (void? (hash-ref r 'result)))))
          results))

;; Their results.
(define (lsp/answers results)
  (map (lambda (r) (hash-ref r 'result)) (lsp/answered results)))

;; Whether no server in an `lsp-request-all!` answer gave a result, errors aside.
(define (lsp/none-answered? results)
  (null? (filter (lambda (r) (not (hash-ref r 'err))) results)))

;; Reports each server's error in an `lsp-request-all!` answer.
(define (lsp/report-answer-errors! what results)
  (for-each (lambda (r) (when (hash-ref r 'err) (lsp/report-error! what (hash-ref r 'err))))
            results))

;; ── Pane resolution ──────────────────────────────────────────────────────────

(define (lsp/resolve-pane pane)
  (let ((panes (buffer-panes pane)))
    (if (null? panes) #f (car panes))))

;; ── Viewport ────────────────────────────────────────────────────────────────

(define (lsp/visible-lines pane)
  (let ((range (viewport-range pane)))
    (- (hash-ref range 'end) (hash-ref range 'start))))

;; ── Position formatting ─────────────────────────────────────────────────────

(define (lsp/format-position line col)
  (string-append (number->string (+ 1 line)) ":" (number->string (+ 1 col))))
