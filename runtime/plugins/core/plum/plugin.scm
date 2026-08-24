;;; HUME's PLUgin Manager — installs/updates ordinary plugins and tree-sitter
;;; grammars. LSP server install/uninstall/registration is core:lsp's own
;;; responsibility. See README.md "File layout".
;;;
;;; Depends on core:stdlib (grammar/plugin install and cleanup call
;;; stdlib/find, stdlib/write-file, stdlib/delete-dir, stdlib/delete-file via
;;; call!) — load it first, same as core:lsp.

(require "plugins.scm")
(require "grammars.scm")
