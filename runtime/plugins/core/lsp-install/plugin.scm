;;; core:lsp-install — plugin.scm (see README.md).

(require "lib/catalog.scm")
(require "lib/register.scm")
(require "lib/discovery.scm")

(lsp-install/register-installed-servers!)
