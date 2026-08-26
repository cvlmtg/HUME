;;; core:lsp/sighelp.scm — textDocument/signatureHelp. See README.md "How it
;;; works" → "Signature help". Dismiss on mode change is handled by
;;; lib.scm's shared `on-mode-change` popup registration.

(require "lib.scm")

;;; A `SignatureHelp.parameters[].label` is a plain string, or a `[start,
;;; end)` offset pair into the signature's own label — the offsets count
;;; code units in the server's negotiated encoding, so the host slices the
;;; label; `#f` back means no attached server to name that encoding.
(define (lsp/param-text bid sig-label param)
  (let ((param-label (hash-ref param "label")))
    (if (string? param-label)
        param-label
        (lsp-label-offsets->text bid sig-label param-label))))

;;; Clamp a server-sent index into `[0, (length lst) - 1]` rather than
;;; trusting it verbatim.
(define (lsp/clamp-index idx lst)
  (max 0 (min idx (- (length lst) 1))))

;;; The chosen signature's label, plus the active parameter's own text
;;; marked with `⟨…⟩` on a second line (no styling API in `show-popup!`
;;; v1).
(define (lsp/sighelp-text bid sig active-idx)
  (let* ((label (hash-ref sig "label"))
         (params (if (hash-contains? sig "parameters") (hash-ref sig "parameters") (list))))
    (if (or (not active-idx) (null? params))
        label
        (let* ((idx (lsp/clamp-index active-idx params))
               (text (lsp/param-text bid label (list-ref params idx))))
          (if text (string-append label "\n⟨" text "⟩") label)))))

;;; `res`: a `SignatureHelp` — `signatures: []` is spec-valid ("nothing to
;;; show"), same as a null/void response at the call site.
(define (lsp/show-sighelp bid res)
  (let ((sigs (hash-ref res "signatures")))
    (if (null? sigs)
        (close-popup!)
        (let* ((active-sig-idx (if (hash-contains? res "activeSignature") (hash-ref res "activeSignature") 0))
               (idx (lsp/clamp-index active-sig-idx sigs))
               (sig (list-ref sigs idx))
               (active-param-idx (if (hash-contains? res "activeParameter") (hash-ref res "activeParameter") #f)))
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

;;; ")" is a dismiss trigger, not a request trigger — still needs
;;; registering or it never reaches Insert-mode text.
(lsp/setup-trigger-chars! "signatureHelpProvider" "lsp-sighelp" (list ")")
  (lambda (bid ch)
    (if (equal? ch ")")
        (close-popup!)
        ;; Guarded so a stale trigger char left registered past detach
        ;; skips politely instead of hitting lsp-request's server-
        ;; resolution failure on every matching keystroke.
        (lsp/guard-capability "signatureHelpProvider"
          (lambda () (lsp/sighelp-request bid))))))
