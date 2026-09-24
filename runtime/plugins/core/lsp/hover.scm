;;; core:lsp/hover.scm — textDocument/hover. See docs/features.md.

(require "lib.scm")

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
           (json-contains? contents "kind")
           (equal? (json-ref contents "kind") "plaintext"))
      #f
      "markdown"))

;; ── Popup: cursor or docked ──────────────────────────────────────────────────

(define (lsp/show-hover pane text lang)
  (let* ((threshold (quotient (lsp/visible-lines pane) 3))
         (lines (split-many text "\n")))
    (if (<= (length lines) threshold)
        (show-popup! pane text #:kind 'scrollable #:lang lang)
        (show-popup! pane text #:kind 'scrollable #:lang lang #:anchor 'bottom))))

;; ── Command ─────────────────────────────────────────────────────────────────

(define-command! "lsp-hover" "Show hover info for the symbol under the cursor."
  (lambda (pane)
    (close-popup!)
    (lsp/guard-capability pane "hoverProvider"
      (lambda ()
        (lsp-request pane "textDocument/hover" (lsp-position-params pane)
          (lambda (err res)
            (cond
              (err (lsp/report-error "hover" err))
              ((void? res) (log! 'info "No hover info"))
              (else (let ((contents (json-ref res "contents")))
                      (lsp/show-hover pane (lsp/hover-contents->text contents)
                                       (lsp/hover-lang contents))))))
          #:allow-stale #t
          #:require-focus #t)))))
