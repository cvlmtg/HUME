;;; core:lsp-install/manifest.scm — see README.md.
(declare-plugin! "core:lsp-install"
  #:languages '("*"))
(declare-plugin! "core:lsp-install"
  #:entry "commands.scm"
  #:commands '("lsp-install/install-server")
  #:typed-commands '("lsp-install" "lsp-uninstall" "lsp-catalog"))
