;;; core:stdlib/manifest.scm — see README.md.
(declare-plugin! "core:stdlib"
  #:commands '("stdlib/primary-selection"
               "stdlib/all-single-char?" "stdlib/single-selection?" "stdlib/cursor-char-offset"
               "stdlib/find" "stdlib/concat" "stdlib/write-file!" "stdlib/delete-dir!" "stdlib/delete-file!"
               "stdlib/list-subdirs"
               "stdlib/git-repo?" "stdlib/git-toplevel" "stdlib/resolve-lang-arg"
               "stdlib/report-section" "stdlib/show-report!"
               "stdlib/config-boolean" "stdlib/config-string" "stdlib/config-enum"
               "stdlib/config-integer" "stdlib/config-list"
               "stdlib/with-tab" "stdlib/with-vsplit" "stdlib/with-split" "stdlib/buffer-actions"
               "stdlib/split-words"))
