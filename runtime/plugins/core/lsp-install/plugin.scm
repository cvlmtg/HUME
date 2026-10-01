;;; core:lsp-install — plugin.scm (see README.md).

(require "register.scm")
(require "commands.scm")

(unless (member "core:stdlib" (declared-plugins))
  (error "core:lsp-install: requires core:stdlib — (declare-plugin! \"core:stdlib\") or (load-plugin! \"core:stdlib\") before (load-plugin! \"core:lsp-install\")"))

(lsp-install/register-installed-servers!)
