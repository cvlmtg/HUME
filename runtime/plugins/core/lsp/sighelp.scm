;;; core:lsp/sighelp.scm — textDocument/signatureHelp. See docs/features.md.

(require "lib.scm")

(define (lsp/param-text bid sig-label param)
  (let ((param-label (json-ref param "label")))
    (if (string? param-label)
        param-label
        (lsp-label-offsets->text bid sig-label param-label))))

(define (lsp/clamp-index idx lst)
  (max 0 (min idx (- (length lst) 1))))

(define (lsp/sighelp-text bid sig active-idx)
  (let* ((label (json-ref sig "label"))
         (params-field (json-ref-or sig #f "parameters"))
         (params (if params-field (json-list params-field) (list))))
    (if (or (not active-idx) (null? params))
        label
        (let* ((idx (lsp/clamp-index active-idx params))
               (text (lsp/param-text bid label (list-ref params idx))))
          (if text (string-append label "\n⟨" text "⟩") label)))))

(define (lsp/show-sighelp bid res)
  (let ((sigs (json-list (json-ref res "signatures"))))
    (if (null? sigs)
        (close-popup!)
        (let* ((active-sig-idx (json-ref-or res 0 "activeSignature"))
               (idx (lsp/clamp-index active-sig-idx sigs))
               (sig (list-ref sigs idx))
               (active-param-idx (json-ref-or res #f "activeParameter")))
          (show-popup! (lsp/sighelp-text bid sig active-param-idx))))))

(define lsp/sighelp-request
  (debounce 150
    (lambda (bid)
      (lsp-request #f "textDocument/signatureHelp" (lsp-position-params bid)
        (lambda (err res)
          (cond
            (err (lsp/report-error "signature help" err) (close-popup!))
            ((void? res) (close-popup!))
            (else (lsp/show-sighelp bid res))))))))

(lsp/setup-trigger-chars! "signatureHelpProvider" "lsp-sighelp" (list ")")
  (lambda (bid ch)
    (if (equal? ch ")")
        (close-popup!)
        (lsp/guard-capability "signatureHelpProvider"
          (lambda () (lsp/sighelp-request bid))))))
