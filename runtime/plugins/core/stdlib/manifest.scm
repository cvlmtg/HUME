; Default activation for `(declare-plugin "core:stdlib")` with no explicit
; #:commands/#:events/#:languages — see README.md "Usage".
(declare-plugin "core:stdlib"
  #:commands '("stdlib/selection-anchor" "stdlib/selection-head" "stdlib/selection-primary?"
               "stdlib/primary-selection"
               "stdlib/all-single-char?" "stdlib/single-selection?" "stdlib/cursor-char-index"
               "stdlib/find" "stdlib/write-file" "stdlib/delete-dir" "stdlib/delete-file"
               "stdlib/list-subdirs" "stdlib/run" "stdlib/git-repo?" "stdlib/git-toplevel"
               "stdlib/resolve-lang-arg"
               "stdlib/config-boolean" "stdlib/config-string" "stdlib/config-enum"
               "stdlib/config-integer" "stdlib/config-list"))
