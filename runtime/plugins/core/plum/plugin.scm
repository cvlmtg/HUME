;;; HUME's PLUgin Manager — installs/updates ordinary plugins and tree-sitter
;;; grammars. LSP server install/uninstall/registration is core:lsp's own
;;; responsibility (see runtime/plugins/core/lsp/servers.scm and
;;; registration.scm).
;;;
;;; The grammar source catalog and startup registration of already-compiled
;;; grammars live in core (runtime/scheme/grammars.scm, evaluated at the VM
;;; top level before any plugin loads) — grammars.scm here calls that file's
;;; bindings (*grammar-sources*, grammar-output-path, etc.) directly for its
;;; install pipeline, with no `require` of its own.

(require "plugins.scm")
(require "grammars.scm")
