;;; core:lsp/inlay.scm — textDocument/inlayHint.
;;;
;;; Off by default — `:set global lsp.inlay-hints=true` opts in.

(require "lib.scm")

;;; `hint`'s label, resolved to plain text — either a bare string or an
;;; `InlayHintLabelPart[]` (each part's "value" concatenated in order).
(define (lsp/inlay-hint-text hint)
  (let ((label (hash-ref hint "label")))
    (if (string? label)
        label
        (string-join (map (lambda (part) (hash-ref part "value")) label) ""))))

;;; `InlayHint {position, label, paddingLeft?, paddingRight?}` -> the
;;; `(position text 'before)` shape `set-inlay-hints!` expects — `position`
;;; passed through as the raw wire hashmap (the setter converts it);
;;; padding becomes literal leading/trailing spaces.
(define (lsp/hint->store-entry hint)
  (let* ((text (lsp/inlay-hint-text hint))
         (pad-left (and (hash-contains? hint "paddingLeft") (equal? (hash-ref hint "paddingLeft") #t)))
         (pad-right (and (hash-contains? hint "paddingRight") (equal? (hash-ref hint "paddingRight") #t)))
         (text (if pad-left (string-append " " text) text))
         (text (if pad-right (string-append text " ") text)))
    (list (hash-ref hint "position") text 'before)))

;;; `(first last)` visible lines -> `InlayHintParams` — `"textDocument"`
;;; from `lsp-position-params`, a hand-built range with character 0 at both
;;; ends (encoding-safe: no wire math needed at a line boundary).
(define (lsp/inlay-hint-params bid first last)
  (hash "textDocument" (hash-ref (lsp-position-params bid) "textDocument")
        "range" (hash "start" (hash "line" first "character" 0)
                       "end" (hash "line" (+ last 1) "character" 0))))

;;; Debounced (200ms) and re-run from both on-viewport-change and
;;; on-diagnostics-changed — servers refresh hints roughly when diagnostics
;;; do, and neither hook carries a stale-safe reason to skip the other.
;;; Always re-reads the current viewport from lib.scm's tracker rather than
;;; trusting an argument, so both call sites can share one signature; `#f`
;;; before the first on-viewport-change event skips silently (the next
;;; viewport event refreshes).
(define lsp/refresh-hints
  (debounce 200
    (lambda (bid)
      (let ((range (lsp/viewport-range bid)))
        (when (and range (get-option "lsp.inlay-hints") (lsp/supports? "inlayHintProvider"))
          (lsp-request #f "textDocument/inlayHint" (lsp/inlay-hint-params bid (car range) (cadr range))
            (lambda (err res)
              (when (and (not err) (not (void? res)) (not (null? res)))
                (set-inlay-hints! bid (map lsp/hint->store-entry res))))))))))

(register-hook! 'on-viewport-change
  (lambda (bid first last) (lsp/refresh-hints bid)))

(register-hook! 'on-diagnostics-changed
  (lambda (bid) (lsp/refresh-hints bid)))

;;; Once `bid` has no attached server, `lsp/refresh-hints` silently skips
;;; (no capability to check), so stale hints from the detached server would
;;; otherwise sit rendered forever. `on-lsp-detach` is the only signal for
;;; this — clear explicitly rather than let them drift.
(register-hook! 'on-lsp-detach
  (lambda (bid server-name) (set-inlay-hints! bid '())))
