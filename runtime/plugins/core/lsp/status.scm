;;; core:lsp/status.scm — server status, stop and restart commands. See docs/architecture.md.

(define-typed-command! "lsp-status"
  "Show registered LSP servers and attached buffers' diagnostic counts."
  (lambda (pane) (lsp-show-status! pane)))

(define-typed-command! "lsp-stop"
  "Stop an LSP server: :lsp-stop [language] (default: this buffer's server)."
  (lambda (pane arg) (lsp-stop! (if (string? arg) arg pane))))

(define-typed-command! "lsp-restart"
  "Restart an LSP server: :lsp-restart [language] (default: this buffer's server)."
  (lambda (pane arg) (lsp-restart! (if (string? arg) arg pane))))
