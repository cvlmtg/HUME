; Default activation for `(declare-plugin "core:git-diff")` with no
; explicit #:commands/#:events/#:languages — see README.md "Usage" and
; "Customizing activation".
(declare-plugin "core:git-diff"
  #:events '(on-buffer-open)
  #:commands '("toggle-git-signs" "toggle-inline-diff"))
