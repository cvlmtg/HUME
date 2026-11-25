;;; core:markdown/manifest.scm — see README.md.
(declare-plugin! "core:markdown"
  #:languages '("markdown")
  #:commands '("markdown/toggle-preview" "format-text/markdown"
               "markdown/enter" "markdown/open-below" "markdown/open-above"
               "markdown/toggle-task" "markdown/bold" "markdown/italic" "markdown/code"
               "markdown/strikethrough" "markdown/follow-link" "markdown/select-cell"
               "markdown/select-around-cell" "markdown/select-column"
               "markdown/move-row-up" "markdown/move-row-down"
               "markdown/move-column-left" "markdown/move-column-right")
  #:typed-commands '("toggle-markdown-preview"))
