;;; core:lsp-install/discovery.scm — see README.md.

(require "catalog.scm")
(require "receipts.scm")
(require "register.scm")
(require "blocker.scm")

(define *lsp-install-hinted-languages* (hash))

(register-hook! 'on-language-set
  (lambda (pane lang)
    (when (and (not (equal? lang "")) (not (hash-contains? *lsp-install-hinted-languages* lang)))
      (set! *lsp-install-hinted-languages* (hash-insert *lsp-install-hinted-languages* lang #t))
      (when (hash-contains? lsp-install/language-lists lang)
        (let ((name (lsp-install/primary lang)))
          (when (and (null? (lsp-language-servers lang))
                     (not (lsp-install/read-receipt name))
                     (not (lsp-install/install-blocker name)))
            (log! 'warn (string-append "LSP: language server '" name
                                       "' is available for " lang " — run :lsp-install"))))))))
