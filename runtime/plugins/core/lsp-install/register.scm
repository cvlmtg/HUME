;;; core:lsp-install/register.scm — see README.md.

(require "catalog.scm")
(require "receipts.scm")

(provide lsp-install/register-installed-servers!)

(define (lsp-install/register-server-languages! name cmd env)
  (let* ((fields (hash-ref (lsp-install/servers-catalog) name))
         (langs  (filter (lambda (lang-entry) (not (lsp-registered-for-language? (car lang-entry))))
                         (cdr (lsp-install/field fields 'languages))))
         (args        (cdr (lsp-install/field fields 'args)))
         (config-json (cdr (lsp-install/field fields 'config)))
         (config (if (null? config-json) #f (json-parse config-json))))
    (for-each
      (lambda (lang-entry)
        (register-lsp-server! (car lang-entry)
                              #:command cmd
                              #:args args
                              #:root-markers (cdr lang-entry)
                              #:init-options config
                              #:settings config
                              #:env env))
      langs)))

(define (lsp-install/register-installed-servers!)
  (let ((sdir (lsp-install/servers-dir)))
    (when (path-exists? sdir)
      (for-each
        (lambda (name)
          (let ((receipt (lsp-install/read-receipt name)))
            (cond
              ((not receipt)
               (log! 'warn (string-append "LSP: interrupted install of " name
                                          " — run :lsp-install to redo, or delete the directory")))
              ((not (hash-contains? (lsp-install/servers-catalog) name))
               (log! 'warn (string-append "LSP: orphan server " name
                                          " — not in the seeded catalog, run :lsp-uninstall to remove")))
              (else
               (lsp-install/register-server-languages!
                 name
                 (path-join (lsp-install/server-dir name) (lsp-install/receipt-bin receipt))
                 (map (lambda (entry)
                        (cons (car entry) (path-join (lsp-install/server-dir name) (cdr entry))))
                      (lsp-install/receipt-env-dirs receipt)))))))
        (call! "stdlib/list-subdirs" sdir)))))
