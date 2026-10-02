;;; core:lsp/manifest.scm — see README.md.
(declare-plugin! "core:lsp"
  #:languages '("*")
  #:commands '("lsp-hover" "lsp-goto-definition" "lsp-goto-declaration"
               "lsp-goto-type-definition" "lsp-goto-implementation" "lsp-references"
               "goto-next-diagnostic" "goto-prev-diagnostic"
               "lsp-rename" "lsp-fmt" "lsp-code-actions")
  #:typed-commands '("diagnostics" "format-source"
                      "lsp-status" "lsp-stop" "lsp-restart"))
