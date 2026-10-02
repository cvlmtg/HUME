;;; core:lsp-install — plugin.scm (see README.md).

(require "catalog.scm")
(require "register.scm")
(require "discovery.scm")

(lsp-install/require-stdlib!)

(lsp-install/register-installed-servers!)
