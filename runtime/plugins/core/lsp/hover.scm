;;; core:lsp/hover.scm — textDocument/hover. See README.md "How it works" →
;;; "Hover".

(require "lib.scm")

;; ── Response decoding ───────────────────────────────────────────────────────

;;; A `MarkedString` is a bare string or `{language, value}`; a
;;; `MarkupContent` is `{kind, value}` — told apart by key.
(define (lsp/marked-string->text ms)
  (cond
    ((string? ms) ms)
    ((hash-contains? ms "language")
     (string-append "```" (hash-ref ms "language") "\n" (hash-ref ms "value") "\n```"))
    (else (hash-ref ms "value"))))

;;; `contents` is a `MarkupContent`, a `MarkedString`, or `MarkedString[]` —
;;; decoded to raw text.
(define (lsp/hover-contents->text contents)
  (cond
    ((string? contents) contents)
    ((list? contents) (string-join (map lsp/marked-string->text contents) "\n\n"))
    (else (lsp/marked-string->text contents))))

;;; The grammar name to highlight `contents` through, or `#f` for plain
;;; text — see README's "Hover" for the plaintext-opt-out rule.
(define (lsp/hover-lang contents)
  (if (and (hash? contents)
           (hash-contains? contents "kind")
           (equal? (hash-ref contents "kind") "plaintext"))
      #f
      "markdown"))

;; ── Popup: cursor or docked ──────────────────────────────────────────────────

;;; Threshold = ⅓ of the last-known viewport height, falling back to 15
;;; lines before the first on-viewport-change event.
(define (lsp/show-hover text lang)
  (let* ((bid (current-buffer))
         (visible (lsp/visible-lines bid))
         (threshold (if visible (quotient visible 3) 15))
         (lines (split-many text "\n")))
    (if (<= (length lines) threshold)
        (show-popup! text #:kind 'scrollable #:lang lang)
        (show-popup! text #:kind 'scrollable #:lang lang #:anchor 'bottom))))

;; ── Dismiss ─────────────────────────────────────────────────────────────────
;; Shared with sighelp.scm — see lib.scm's `on-mode-change` registration.

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
              (else (let ((contents (hash-ref res "contents")))
                      (lsp/show-hover (lsp/hover-contents->text contents)
                                       (lsp/hover-lang contents))))))
          #:allow-stale #t)))))
