;;; core:lsp/hover.scm — textDocument/hover. See docs/features.md.

(require "helpers.scm")

;; ── Response decoding ───────────────────────────────────────────────────────

(define (lsp/marked-string->text ms)
  (if (string? ms)
      ms
      (let ((lang (json-ref-or ms #f "language")))
        (if lang
            (string-append "```" lang "\n" (json-ref ms "value") "\n```")
            (json-ref ms "value")))))

(define (lsp/hover-contents->text contents)
  (cond
    ((string? contents) contents)
    ((json-array? contents) (string-join (map lsp/marked-string->text (json-list contents)) "\n\n"))
    (else (lsp/marked-string->text contents))))

(define (lsp/hover-lang contents)
  (if (and (json-object? contents)
           (equal? (json-ref-or contents #f "kind") "plaintext"))
      #f
      "markdown"))

;; ── Popup: cursor or docked ──────────────────────────────────────────────────

(define lsp/*hover-popup* #f)

(define (lsp/show-hover pane text lang)
  (set! lsp/*hover-popup* (show-popup! pane text #:kind 'scrollable #:lang lang #:anchor 'fit)))

(define (lsp/close-hover!)
  (close-popup! lsp/*hover-popup*)
  (set! lsp/*hover-popup* #f))

;; ── Command ─────────────────────────────────────────────────────────────────

(define-command! "lsp/hover" "Show hover info for the symbol under the cursor."
  (lambda (pane)
    (lsp/close-hover!)
    (lsp/with-servers pane "hover"
      (lambda (servers)
        (lsp-request! pane "textDocument/hover" (lsp-position-params pane)
          (lambda (err res)
            (cond
              (err (lsp/report-error! "hover" err))
              ((void? res) (log! 'info "No hover info"))
              (else (let ((contents (json-ref res "contents")))
                      (lsp/show-hover pane (lsp/hover-contents->text contents)
                                       (lsp/hover-lang contents))))))
          #:allow-stale #t
          #:require-focus #t))
      #:method "textDocument/hover")))
