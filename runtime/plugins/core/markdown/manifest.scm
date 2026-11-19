;;; core:markdown/manifest.scm — see README.md.
(declare-plugin! "core:markdown"
  #:languages '("markdown")
  #:commands '("markdown/toggle-preview" "format-text/markdown" "lsp-completion/markdown")
  #:typed-commands '("toggle-markdown-preview"))
