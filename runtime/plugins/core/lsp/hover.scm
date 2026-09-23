;;; core:lsp/hover.scm — textDocument/hover. See docs/features.md.

(require "lib.scm")

;; ── Response decoding ───────────────────────────────────────────────────────

;;; `MarkedString` (bare string or `{language, value}`) vs `MarkupContent`
;;; (`{kind, value}`) — told apart by key. `ms` is a native string or a JSON
;;; object handle (`res` — this file's whole response — is one too).
(define (lsp/marked-string->text ms)
  (cond
    ((string? ms) ms)
    ((json-contains? ms "language")
     (string-append "```" (json-ref ms "language") "\n" (json-ref ms "value") "\n```"))
    (else (json-ref ms "value"))))

(define (lsp/hover-contents->text contents)
  (cond
    ((string? contents) contents)
    ((json-array? contents) (string-join (map lsp/marked-string->text (json-list contents)) "\n\n"))
    (else (lsp/marked-string->text contents))))

;;; Grammar name to highlight through, or `#f` for plain text.
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
              ;; JSON null decodes to Steel void, not #f.
              ((void? res) (log! 'info "No hover info"))
              (else (let ((contents (json-ref res "contents")))
                      (lsp/show-hover (lsp/hover-contents->text contents)
                                       (lsp/hover-lang contents))))))
          #:allow-stale #t)))))
