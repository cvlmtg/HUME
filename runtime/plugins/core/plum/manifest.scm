; Default activation for `(declare-plugin "core:plum")` with no explicit
; #:commands/#:typed-commands/#:events/#:languages — see README.md "Usage".
; Command-triggered only: grammar registration is core's job
; (runtime/scheme/grammars.scm).
(declare-plugin "core:plum"
  #:commands '("plum-ensure-grammars")
  #:typed-commands '("plum-install-plugins" "plum-cleanup-plugins" "plum-update-plugins" "plum-list-plugins"
                      "plum-install-grammar"
                      "plum-list-grammars" "plum-cleanup-grammars"))
