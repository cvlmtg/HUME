;;; core:markdown/manifest.scm — see README.md.
(declare-plugin! "core:markdown"
  #:languages '("markdown")
  #:commands '("markdown/toggle-preview" "format-text/markdown"
               "markdown/enter" "markdown/open-below" "markdown/open-above"
               "markdown/toggle-task" "markdown/select-cell"
               "markdown/select-around-cell" "markdown/select-column")
  #:typed-commands '("toggle-markdown-preview"))
