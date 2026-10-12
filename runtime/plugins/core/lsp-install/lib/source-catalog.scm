;;; core:lsp-install/source-catalog.scm — see README.md.

(require "catalog.scm")

(provide lsp-install/source lsp-install/source-names)

(define (lsp-install/source name)
  (lsp-install/read-record "sources" name))

(define (lsp-install/source-names)
  (data-file-names (lsp-install/data-path "sources")))
