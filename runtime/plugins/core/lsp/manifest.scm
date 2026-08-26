; Default activation for `(declare-plugin "core:lsp")` with no explicit
; #:commands/#:events/#:languages — see README.md "Usage".
(declare-plugin "core:lsp"
  #:languages '("*")
  #:commands '("lsp-hover" "lsp-goto-definition" "lsp-goto-declaration"
               "lsp-goto-type-definition" "lsp-goto-implementation" "lsp-references"
               "goto-next-diagnostic" "goto-prev-diagnostic" "diagnostics"
               "lsp-rename" "lsp-fmt" "lsp-code-actions" "lsp-completion-trigger"
               "lsp-install" "lsp-uninstall" "lsp-servers" "lsp-rescan-servers"
               "lsp-status" "lsp-stop" "lsp-restart"))
