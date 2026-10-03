;;; core:plum/manifest.scm — see README.md.
(declare-plugin! "core:plum"
  #:typed-commands '("plum-install-plugins" "plum-cleanup-plugins" "plum-update-plugins" "plum-list-plugins"))
(declare-plugin! "core:plum"
  #:entry "grammars.scm"
  #:commands '("plum-ensure-grammars")
  #:typed-commands '("plum-install-grammar" "plum-list-grammars" "plum-cleanup-grammars"))
(declare-plugin! "core:plum"
  #:entry "themes.scm"
  #:typed-commands '("plum-install-theme" "plum-update-themes" "plum-list-themes" "plum-remove-theme"))
