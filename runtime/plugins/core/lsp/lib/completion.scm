;;; core:lsp/completion.scm — textDocument/completion. See docs/features.md.

(require "helpers.scm")

;; ── The source ───────────────────────────────────────────────────────────────

(define lsp/completion-key (lsp-supersede-key))

;; See docs/features.md, "Completion".
(define (lsp/completion-answer pane r)
  (let ([rewrite (string-append "lsp-completion/" (get-buffer-option pane "language"))])
    (if (command-exists? rewrite)
        (call! rewrite pane (hash-ref r 'server) (hash-ref r 'result))
        (hash-ref r 'result))))

(register-completion-source! "lsp"
  (lambda (id pane prefix)
    (lsp-request-all! pane "textDocument/completion" (lsp-position-params pane)
      (lambda (err results)
        (lsp/report-answer-errors! "completion" results)
        (completion-emit! id (map (lambda (r) (lsp/completion-answer pane r))
                                  (lsp/answered results))))
      #:unavailable 'empty
      #:supersede lsp/completion-key))
  #:target 'buffer #:priority 10 #:resolve #t)

;; ── Trigger chars ─────────────────────────────────────────────────────────────

(lsp/setup-trigger-chars! 'completion "lsp" '() #f)
