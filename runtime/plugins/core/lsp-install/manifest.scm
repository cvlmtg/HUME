; Default activation for `(declare-plugin! "core:lsp-install")` with no explicit
; #:commands/#:typed-commands/#:events/#:languages — see README.md.
(declare-plugin! "core:lsp-install"
  #:languages '("*")
  #:typed-commands '("lsp-install" "lsp-uninstall" "lsp-servers" "lsp-rescan-servers"))
