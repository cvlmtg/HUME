;;; core:lsp/sighelp.scm — textDocument/signatureHelp. See docs/features.md.

(require "lib.scm")

;;; `param-label` (when not a plain string) is `(json-ref param "label")` —
;;; itself a child of the signature-help response, so it carries that
;;; response's own tagged producing-server encoding straight into
;;; `lsp-label-offsets->text`, with no pane needed here at all.
(define (lsp/param-text sig-label param)
  (let ((param-label (json-ref param "label")))
    (if (string? param-label)
        param-label
        (lsp-label-offsets->text sig-label param-label))))

(define (lsp/clamp-index idx lst)
  (max 0 (min idx (- (length lst) 1))))

(define (lsp/sighelp-text sig active-idx)
  (let* ((label (json-ref sig "label"))
         (params-field (json-ref-or sig #f "parameters"))
         (params (if params-field (json-list params-field) (list))))
    (if (or (not active-idx) (null? params))
        label
        (let* ((idx (lsp/clamp-index active-idx params))
               (text (lsp/param-text label (list-ref params idx))))
          (if text (string-append label "\n⟨" text "⟩") label)))))

;;; `pane` is the request's own invocation pane — `#:require-focus #t`
;;; below guarantees it's still focused when this runs, so it's the right
;;; pane to anchor the popup at.
(define (lsp/show-sighelp pane res)
  (let ((sigs (json-list (json-ref res "signatures"))))
    (if (null? sigs)
        (close-popup!)
        (let* ((active-sig-idx (json-ref-or res 0 "activeSignature"))
               (idx (lsp/clamp-index active-sig-idx sigs))
               (sig (list-ref sigs idx))
               (active-param-idx (json-ref-or res #f "activeParameter")))
          (show-popup! pane (lsp/sighelp-text sig active-param-idx))))))

(define lsp/sighelp-request
  (debounce 150
    (lambda (pane)
      ;; `pane` itself — not just its buffer — may have closed, or been
      ;; switched to another buffer, during the debounce window:
      ;; `lsp-position-params` resolves the pane (not just the buffer) and
      ;; would raise on either. A benign "this pane is no longer what it
      ;; was when the keystroke armed this timer" is not worth a logged
      ;; error.
      (when (pane-live? pane)
        (lsp-request pane "textDocument/signatureHelp" (lsp-position-params pane)
          (lambda (err res)
            (cond
              (err (lsp/report-error "signature help" err) (close-popup!))
              ((void? res) (close-popup!))
              (else (lsp/show-sighelp pane res))))
          #:require-focus #t)))))

(lsp/setup-trigger-chars! "signatureHelpProvider" "lsp-sighelp" (list ")")
  (lambda (pane ch)
    (if (equal? ch ")")
        (close-popup!)
        (lsp/guard-capability pane "signatureHelpProvider"
          (lambda () (lsp/sighelp-request pane))))))
