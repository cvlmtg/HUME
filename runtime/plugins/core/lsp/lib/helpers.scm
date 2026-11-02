;;; core:lsp/lib/helpers.scm — shared helpers used by every feature file. See
;;; docs/architecture.md.

(provide lsp/report-error! lsp/answered lsp/answers lsp/answer-items lsp/none-answered? lsp/report-answer-errors!
         lsp/on-answers lsp/with-params
         lsp/setup-trigger-chars! lsp/format-position lsp/cap-flag?
         lsp/with-servers lsp/report-unsupported lsp/jump-to! lsp/payload-rows lsp/refresh-drawer-list!)

;; ── Capabilities ────────────────────────────────────────────────────────────

;; `cap` is a server's `(lsp-capability …)`: `#t`, an options object, or `#f`.
(define (lsp/cap-field cap field default)
  (if (json-object? cap) (json-ref-or cap default field) default))

(define (lsp/cap-flag? cap field)
  (equal? (lsp/cap-field cap field #f) #t))

;; Calls `(proceed params)` with `params`, a pane's `lsp-position-params` or
;; `lsp-document-params`, or logs that the buffer has no file when it has none.
(define (lsp/with-params params what proceed)
  (if params
      (proceed params)
      (log! 'info (string-append "lsp " what ": this buffer has no file"))))

;; Logs that `what` is unsupported by this buffer's language servers.
(define (lsp/report-unsupported what)
  (log! 'info (string-append what " is not supported by this buffer's language servers")))

;; Calls `(proceed servers)` with `servers`, or reports `what` unsupported when empty.
(define (lsp/with-servers servers what proceed)
  (if (null? servers)
      (lsp/report-unsupported what)
      (proceed servers)))

;; Jumps the focused pane to `location`, a `goto-location!` target, and logs
;; when the place it names no longer exists.
(define (lsp/jump-to! location)
  (unless (goto-location! (focused-pane) location)
    (log! 'warning "lsp: the location no longer exists")))

(define (lsp/payload-rows row-text items)
  (map (lambda (item) (cons (row-text item) item)) items))

;; Replaces the rows of the drawer `token` names with `items`, moving its
;; selection to `(pick-index selected)`. Returns #f when that drawer has
;; closed or another has replaced it.
(define (lsp/refresh-drawer-list! token row-text items on-select pick-index)
  (let ((selected (drawer-selected-index token)))
    (and selected
         (update-drawer-list! token (lsp/payload-rows row-text items) on-select
                              (pick-index selected)))))

;; ── Trigger-char lifecycle ──────────────────────────────────────────────────

;;; A server's `triggerCharacters` strings, as chars.
(define (lsp/trigger-chars strings)
  (map (lambda (s)
         (if (= (string-length s) 1)
             (string-ref s 0)
             (error (string-append "lsp: trigger character \"" s "\" is not one character"))))
       strings))

(define (lsp/setup-trigger-chars! feature source-name extra-chars on-trigger)
  (define (set-chars! pane server chars)
    (if on-trigger
        (set-attachment-hook-triggers! pane source-name server feature chars)
        (set-attachment-completion-triggers! pane source-name server feature chars)))
  (register-hook! 'on-lsp-attach
    (lambda (pane server)
      (let ((tc (lsp/cap-field (lsp-capability server #:feature feature) "triggerCharacters" #f)))
        (set-chars! pane server
                    (append extra-chars (if tc (lsp/trigger-chars (json-list tc)) (list)))))))
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

;; The elements of every server's array answer, in server order.
(define (lsp/answer-items results)
  (call! "stdlib/concat" (map json-list (lsp/answers results))))

;; Whether an `lsp-request-all!` answer has entries and every one is an error.
(define (lsp/none-answered? results)
  (and (pair? results)
       (not (call! "stdlib/find" (lambda (r) (not (hash-ref r 'err))) results))))

;; Reports each server's error in an `lsp-request-all!` answer.
(define (lsp/report-answer-errors! what results)
  (for-each (lambda (r) (when (hash-ref r 'err) (lsp/report-error! what (hash-ref r 'err))))
            results))

;; An `lsp-request-all!` callback: reports the request's error, or reports
;; each server's error and calls `(proceed results)`.
(define (lsp/on-answers what proceed)
  (lambda (err results)
    (if err
        (lsp/report-error! what err)
        (begin
          (lsp/report-answer-errors! what results)
          (proceed results)))))

;; ── Position formatting ─────────────────────────────────────────────────────

(define (lsp/format-position line col)
  (string-append (number->string (+ 1 line)) ":" (number->string (+ 1 col))))
