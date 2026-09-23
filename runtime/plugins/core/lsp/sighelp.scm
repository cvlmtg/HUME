;;; core:lsp/sighelp.scm — textDocument/signatureHelp. See docs/features.md.

(require "lib.scm")

;;; `param-label` is a native string, or a JSON array handle for the raw
;;; `[start, end)` offset pair — `lsp-label-offsets->text` accepts either
;;; shape directly. `#f` back means no attached server to name the
;;; label-offset encoding.
(define (lsp/param-text bid sig-label param)
  (let ((param-label (json-ref param "label")))
    (if (string? param-label)
        param-label
        (lsp-label-offsets->text bid sig-label param-label))))

(define (lsp/clamp-index idx lst)
  (max 0 (min idx (- (length lst) 1))))

;;; `params`/`sigs` (below) are unpacked to Steel lists via `json-list` —
;;; `lsp/clamp-index`/`list-ref`/`null?` are list operations, not JSON handle
;;; ones.
(define (lsp/sighelp-text bid sig active-idx)
  (let* ((label (json-ref sig "label"))
         (params (if (json-contains? sig "parameters") (json-list (json-ref sig "parameters")) (list))))
    (if (or (not active-idx) (null? params))
        label
        (let* ((idx (lsp/clamp-index active-idx params))
               (text (lsp/param-text bid label (list-ref params idx))))
          (if text (string-append label "\n⟨" text "⟩") label)))))

(define (lsp/show-sighelp bid res)
  (let ((sigs (json-list (json-ref res "signatures"))))
    (if (null? sigs)
        (close-popup!)
        (let* ((active-sig-idx (if (json-contains? res "activeSignature") (json-ref res "activeSignature") 0))
               (idx (lsp/clamp-index active-sig-idx sigs))
               (sig (list-ref sigs idx))
               (active-param-idx (if (json-contains? res "activeParameter") (json-ref res "activeParameter") #f)))
          (show-popup! (lsp/sighelp-text bid sig active-param-idx))))))

(define lsp/sighelp-request
  (debounce 150
    (lambda (bid)
      (lsp-request #f "textDocument/signatureHelp" (lsp-position-params bid)
        (lambda (err res)
          (cond
            (err (lsp/report-error "signature help" err) (close-popup!))
            ((void? res) (close-popup!))
            (else (lsp/show-sighelp bid res))))
        ))))

;;; ")" is a dismiss trigger, not a request trigger.
(lsp/setup-trigger-chars! "signatureHelpProvider" "lsp-sighelp" (list ")")
  (lambda (bid ch)
    (if (equal? ch ")")
        (close-popup!)
        ;; Guards against a stale trigger char left registered past detach.
        (lsp/guard-capability "signatureHelpProvider"
          (lambda () (lsp/sighelp-request bid))))))
