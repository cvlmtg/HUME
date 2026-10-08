;;; core:lsp-install/source-catalog.scm — see README.md.

(require "catalog.scm")

(provide lsp-install/source)

(define lsp-install/sources (lsp-install/index-entries (lsp-install/read-data "sources.scm")))

(define (lsp-install/source name)
  (lsp-install/lookup lsp-install/sources name))
