;;; core:plum/manifest.scm — see README.md.
(declare-plugin! "core:plum"
  #:commands '("plum-ensure-grammars")
  #:typed-commands '("plum-install-plugins" "plum-cleanup-plugins" "plum-update-plugins" "plum-list-plugins"
                      "plum-install-grammar"
                      "plum-list-grammars" "plum-cleanup-grammars"
                      "plum-install-theme" "plum-update-themes" "plum-list-themes" "plum-remove-theme"))
