;;; core:plum — see README.md.

(unless (member "core:stdlib" (declared-plugins))
  (error "core:plum: requires core:stdlib — (declare-plugin! \"core:stdlib\") or (load-plugin! \"core:stdlib\") before (load-plugin! \"core:plum\")"))

(require "plugins.scm")
(require "grammars.scm")
(require "themes.scm")
