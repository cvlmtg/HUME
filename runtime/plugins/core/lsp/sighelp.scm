;;; core:lsp/sighelp.scm — textDocument/signatureHelp. See docs/features.md.

(require "lib.scm")

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

(define lsp/*sighelp-popup* #f)

(define (lsp/close-sighelp!)
  (when lsp/*sighelp-popup*
    (close-popup! lsp/*sighelp-popup*)
    (set! lsp/*sighelp-popup* #f)))

(define (lsp/show-sighelp pane res)
  (let ((sigs (json-list (json-ref res "signatures"))))
    (if (null? sigs)
        (lsp/close-sighelp!)
        (let* ((active-sig-idx (json-ref-or res 0 "activeSignature"))
               (idx (lsp/clamp-index active-sig-idx sigs))
               (sig (list-ref sigs idx))
               (active-param-idx (json-ref-or res #f "activeParameter")))
          (set! lsp/*sighelp-popup* (show-popup! pane (lsp/sighelp-text sig active-param-idx)))))))

(define lsp/sighelp-request
  (debounce 150
    (lambda (pane)
      (when (pane-live? pane)
        (lsp-request! pane "textDocument/signatureHelp" (lsp-position-params pane)
          (lambda (err res)
            (cond
              (err (lsp/report-error "signature help" err) (lsp/close-sighelp!))
              ((void? res) (lsp/close-sighelp!))
              (else (lsp/show-sighelp pane res))))
          #:require-focus #t)))))

(lsp/setup-trigger-chars! "signatureHelpProvider" "lsp-sighelp" (list ")")
  (lambda (pane ch)
    (if (equal? ch ")")
        (lsp/close-sighelp!)
        (lsp/guard-capability pane "signatureHelpProvider"
          (lambda () (lsp/sighelp-request pane))))))
