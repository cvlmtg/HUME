; Default activation for `(declare-plugin "core:plum")` with no explicit
; #:commands/#:events/#:languages — see README.md "Usage" and "Caveat".
(declare-plugin "core:plum"
  #:languages '("*")
  #:commands '("plum-install" "plum-cleanup" "plum-update" "plum-list"
               "plum-install-grammar" "plum-ensure-grammars"
               "plum-list-grammars" "plum-cleanup-grammars"))
