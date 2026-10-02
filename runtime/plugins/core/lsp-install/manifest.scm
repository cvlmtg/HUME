;;; core:lsp-install/manifest.scm — see README.md.
(declare-plugin! "core:lsp-install"
  #:languages '("*")
  #:typed-commands '("lsp-rescan-servers"))
(declare-plugin! "core:lsp-install"
  #:entry "commands.scm"
  #:typed-commands '("lsp-install" "lsp-uninstall" "lsp-servers"))
