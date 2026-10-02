;;; core:plum — see README.md.

(unless (member "core:stdlib" (declared-plugins))
  (error "core:plum: requires core:stdlib — add (load-plugin! \"core:stdlib\") before (load-plugin! \"core:plum\")"))

(require "plugins.scm")
(require "grammars.scm")
(require "themes.scm")
