;;; HUME's PLUgin Manager — installs/updates ordinary plugins, tree-sitter
;;; grammars, and third-party themes. LSP server install/uninstall/
;;; registration is core:lsp's own responsibility. See README.md "File
;;; layout".
;;;
;;; Depends on core:stdlib (plugin/grammar/theme install and cleanup call
;;; stdlib/find, stdlib/write-file, stdlib/delete-dir, stdlib/delete-file,
;;; stdlib/list-subdirs, stdlib/run, stdlib/resolve-lang-arg via call!) —
;;; load it first, same as core:lsp.

(require "plugins.scm")
(require "grammars.scm")
(require "themes.scm")
