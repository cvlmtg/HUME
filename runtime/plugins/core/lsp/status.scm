;;; core:lsp/status.scm — server status, stop and restart commands. See docs/architecture.md.

(define-typed-command! "lsp-status"
  "Show running LSP servers and attached buffers' servers and diagnostic counts."
  (lambda (pane) (lsp-show-status! pane)))

(define-typed-command! "lsp-stop"
  "Stop an LSP server: :lsp-stop [name] (default: every server on this buffer)."
  (lambda (pane arg) (lsp-stop! (if (string? arg) arg pane))))

(define-typed-command! "lsp-restart"
  "Restart an LSP server: :lsp-restart [name] (default: every server on this buffer)."
  (lambda (pane arg) (lsp-restart! (if (string? arg) arg pane))))
