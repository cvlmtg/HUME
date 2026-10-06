;;; core:lsp-install/source-catalog.scm — see README.md.

(require "catalog.scm")

(provide lsp-install/source lsp-install/server-command)

(define lsp-install/sources (lsp-install/index-entries (lsp-install/read-data "sources.scm")))

(define lsp-install/server-commands (lsp-install/index-entries (lsp-install/read-data "server-commands.scm")))

(define (lsp-install/source name)
  (lsp-install/lookup lsp-install/sources name))

;; A server missing from the file runs a command named like itself.
(define (lsp-install/server-command name)
  (if (hash-contains? lsp-install/server-commands name)
      (hash-ref lsp-install/server-commands name)
      name))
