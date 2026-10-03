;;; core:git-diff/manifest.scm — see README.md.
(declare-plugin! "core:git-diff"
  #:events '(on-buffer-open)
  #:commands '("git-diff/render-diff")
  #:typed-commands '("toggle-git-signs" "toggle-inline-diff"))
