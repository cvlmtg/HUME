;;; core:markdown/manifest.scm — see README.md.
(declare-plugin! "core:markdown"
  #:languages '("markdown")
  #:commands '("markdown/toggle-preview" "format-text/markdown"
               "markdown/enter" "markdown/open-below" "markdown/open-above")
  #:typed-commands '("toggle-markdown-preview"))
