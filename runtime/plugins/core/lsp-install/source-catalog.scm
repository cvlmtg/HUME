;;; core:lsp-install/source-catalog.scm — see README.md.

(require "catalog.scm")

(provide lsp-install/source)

(define lsp-install/sources (lsp-install/index-entries (lsp-install/read-data "sources.scm")))

(define (lsp-install/source name)
  (if (hash-contains? lsp-install/sources name)
      (hash-ref lsp-install/sources name)
      #f))
