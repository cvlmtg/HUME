;;; core:lsp/manifest.scm — see README.md.
(declare-plugin! "core:lsp"
  #:languages '("*")
  #:commands '("lsp/hover" "lsp/goto-definition" "lsp/goto-declaration"
               "lsp/goto-type-definition" "lsp/goto-implementation" "lsp/references"
               "lsp/goto-next-diagnostic" "lsp/goto-prev-diagnostic"
               "lsp/rename" "lsp/format-source" "lsp/format-document" "lsp/code-actions")
  #:typed-commands '("diagnostics" "format-source"
                      "lsp-status" "lsp-stop" "lsp-restart"
                      "toggle-inlay-hints"))
