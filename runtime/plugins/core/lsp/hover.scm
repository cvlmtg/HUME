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

(define (lsp/show-hover text lang)
  (let* ((bid (current-buffer))
         (visible (lsp/visible-lines bid))
         (threshold (if visible (quotient visible 3) 15))
         (lines (split-many text "\n")))
    (if (<= (length lines) threshold)
        (show-popup! text #:kind 'scrollable #:lang lang)
        (show-popup! text #:kind 'scrollable #:lang lang #:anchor 'bottom))))

;; ── Command ─────────────────────────────────────────────────────────────────

(define-command! "lsp-hover" "Show hover info for the symbol under the cursor."
  (lambda ()
    (close-popup!)
    (lsp/guard-capability "hoverProvider"
      (lambda ()
        (lsp-request #f "textDocument/hover" (lsp-position-params (current-buffer))
          (lambda (err res)
            (cond
              (err (lsp/report-error "hover" err))
              ((void? res) (log! 'info "No hover info"))
              (else (let ((contents (json-ref res "contents")))
                      (lsp/show-hover (lsp/hover-contents->text contents)
                                       (lsp/hover-lang contents))))))
          #:allow-stale #t)))))
