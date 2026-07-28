; Default activation for `(declare-plugin "core:plum")` with no explicit
; #:commands/#:events/#:languages — see README.md "Usage". Command-triggered
; only: grammar registration is core's job (runtime/scheme/grammars.scm).
(declare-plugin "core:plum"
  #:commands '("plum-install" "plum-cleanup" "plum-update" "plum-list"
               "plum-install-grammar" "plum-ensure-grammars"
               "plum-list-grammars" "plum-cleanup-grammars"))
