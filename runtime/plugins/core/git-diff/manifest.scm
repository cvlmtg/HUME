; Default activation for `(declare-plugin "core:git-diff")` with no
; explicit #:commands/#:typed-commands/#:events/#:languages — see README.md "Usage".
(declare-plugin "core:git-diff"
  #:events '(on-buffer-open)
  #:typed-commands '("toggle-git-signs" "toggle-inline-diff"))
