;;; core:markdown-preview/manifest.scm — see README.md.
(declare-plugin! "core:markdown-preview"
  #:languages '("markdown")
  #:commands '("markdown-preview/toggle" "format-text/markdown")
  #:typed-commands '("toggle-markdown-preview"))
