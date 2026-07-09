;;; core:lsp/hover.scm — textDocument/hover.

(require "lib.scm")

;; ── Response decoding ───────────────────────────────────────────────────────

;;; A `MarkedString` is either a bare string or `{language, value}`; a
;;; `MarkupContent` is `{kind, value}` — both hashmap forms carry "value", so
;;; no kind/language branch is needed for v1's raw-text rendering.
(define (lsp/marked-string->text ms)
  (if (string? ms) ms (hash-ref ms "value")))

;;; `contents` is a `MarkupContent`, a `MarkedString`, or `MarkedString[]` —
;;; v1 renders markdown as plain text (strip nothing; code fences read fine
;;; in a monospace popup).
(define (lsp/hover-contents->text contents)
  (cond
    ((string? contents) contents)
    ((list? contents) (string-join (map lsp/marked-string->text contents) "\n\n"))
    (else (lsp/marked-string->text contents))))

;; ── Popup / drawer branch ───────────────────────────────────────────────────

;;; Threshold = ⅓ of the last-known viewport height (the popup's ⅓-pane-height
;;; cap), falling back to 15 lines before the first on-viewport-change
;;; event. An occasional over-tall popup just gets truncated by
;;; `wrap_text` — this heuristic never has to be exact.
(define (lsp/show-hover text)
  (let* ((bid (current-buffer))
         (visible (lsp/visible-lines bid))
         (threshold (if visible (quotient visible 3) 15))
         (lines (split-many text "\n")))
    (if (<= (length lines) threshold)
        (show-popup! text)
        (show-drawer-list! lines (lambda (idx) (begin))))))

;; ── Dismiss ─────────────────────────────────────────────────────────────────
;; A stale hover popup must not linger once the user has moved on — close on
;; any mode change (leaving Insert, entering Command, …) in addition to the
;; top-of-next-hover close below. Shared popup widget: harmless no-op
;; when nothing is open, and no other feature owns this popup yet.
(register-hook! 'on-mode-change (lambda (old-mode new-mode) (close-popup!)))

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
              ;; JSON null decodes to Steel void, not #f — `res` is
              ;; never the boolean #f on a successful response.
              ((void? res) (log! 'info "No hover info"))
              (else (lsp/show-hover (lsp/hover-contents->text (hash-ref res "contents"))))))
          #:allow-stale #t)))))
