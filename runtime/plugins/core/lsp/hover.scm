;;; core:lsp/hover.scm — textDocument/hover.

(require "lib.scm")

;; ── Response decoding ───────────────────────────────────────────────────────

;;; A `MarkedString` is either a bare string or `{language, value}`; a
;;; `MarkupContent` is `{kind, value}` — both hashmap forms carry "value", so
;;; no kind/language branch is needed here: `kind` is only consulted by
;;; `lsp/hover-lang` (below), for the popup/drawer's own highlighting.
(define (lsp/marked-string->text ms)
  (if (string? ms) ms (hash-ref ms "value")))

;;; `contents` is a `MarkupContent`, a `MarkedString`, or `MarkedString[]` —
;;; decoded to raw text (strip nothing; code fences read fine either
;;; unhighlighted or through `#:lang`'s injected-language highlighting).
(define (lsp/hover-contents->text contents)
  (cond
    ((string? contents) contents)
    ((list? contents) (string-join (map lsp/marked-string->text contents) "\n\n"))
    (else (lsp/marked-string->text contents))))

;;; The grammar name to highlight `contents` through, or `#f` for plain text.
;;; Only an explicit `MarkupContent` with `kind: "plaintext"` opts out — a
;;; bare `MarkedString`/`MarkedString[]` and `kind: "markdown"` both render
;;; as markdown (per the LSP spec, `MarkedString` content is always
;;; markdown). Data-driven so a future non-markdown `kind` needs no new
;;; parameter on `show-popup!`/`show-drawer-list!`, just another branch here.
(define (lsp/hover-lang contents)
  (if (and (hash? contents)
           (hash-contains? contents "kind")
           (equal? (hash-ref contents "kind") "plaintext"))
      #f
      "markdown"))

;; ── Popup / drawer branch ───────────────────────────────────────────────────

;;; Threshold = ⅓ of the last-known viewport height (the popup's ⅓-pane-height
;;; cap), falling back to 15 lines before the first on-viewport-change
;;; event. An occasional over-tall popup just gets truncated by
;;; `wrap_text` — this heuristic never has to be exact.
(define (lsp/show-hover text lang)
  (let* ((bid (current-buffer))
         (visible (lsp/visible-lines bid))
         (threshold (if visible (quotient visible 3) 15))
         (lines (split-many text "\n")))
    (if (<= (length lines) threshold)
        (show-popup! text #:scroll #t #:lang lang)
        (show-drawer-list! lines (lambda (idx) (begin)) #:lang lang))))

;; ── Dismiss ─────────────────────────────────────────────────────────────────
;; A stale hover popup must not linger once the user has moved on — it closes
;; on any key other than Ctrl+u/Ctrl+d (`#:scroll #t`, which those two page
;; instead), and on any mode change (leaving Insert, entering Command, …) as
;; a backstop for the cases a key press doesn't cover (e.g. a mouse-driven
;; mode switch). The on-mode-change registration lives in lib.scm (shared
;; popup widget — sighelp.scm uses the same close-on-mode-change dismissal,
;; so one registration covers both).

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
              (else (let ((contents (hash-ref res "contents")))
                      (lsp/show-hover (lsp/hover-contents->text contents)
                                       (lsp/hover-lang contents))))))
          #:allow-stale #t)))))
