; Default activation for `(declare-plugin "core:stdlib")` with no explicit
; #:commands/#:events/#:languages — see README.md "Usage".
(declare-plugin "core:stdlib"
  #:commands '("stdlib/all-single-char?" "stdlib/single-selection?" "stdlib/cursor-char-index"))
