;;; core:git-diff/manifest.scm — see README.md.
(declare-plugin! "core:git-diff"
  #:events '(on-buffer-open)
  #:typed-commands '("toggle-git-signs" "toggle-inline-diff"))
