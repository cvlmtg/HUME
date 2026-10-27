;;; core:lsp-install/register.scm — see README.md.

(require "catalog.scm")
(require "receipts.scm")

(provide lsp-install/register-installed-servers!)

(define (lsp-install/register-server! name cmd env)
  (unless (lsp-server-registered? name)
    (let* ((fields (lsp-install/registration name))
           (args        (lsp-install/ref-or fields 'args '()))
           (config-json (lsp-install/ref-or fields 'config #f))
           (config (and config-json (json-parse config-json))))
      (register-lsp-server! name
                            #:command cmd
                            #:args args
                            #:init-options config
                            #:settings config
                            #:env env))))

;; Default lists: see README.md, "Catalogs".
(define (lsp-install/entry->spec entry)
  (if (null? (cdr entry))
      (car entry)
      (hash 'name (car entry) (car (cadr entry)) (cdr (cadr entry)))))

(define (lsp-install/set-default-lists!)
  (for-each
    (lambda (lang)
      (set-default-language-servers!
        lang
        (map lsp-install/entry->spec (lsp-install/language-list lang))))
    (lsp-install/language-names)))

(define (lsp-install/register-installed-servers!)
  (let ((sdir (lsp-install/servers-dir)))
    (when (path-exists? sdir)
      (for-each
        (lambda (name)
          (let ((receipt (lsp-install/read-receipt name)))
            (cond
              ((not receipt)
               (log! 'warning (string-append "LSP: interrupted install of " name
                                          " — run :lsp-install to redo, or delete the directory")))
              ((not (lsp-install/seeded-server? name))
               (log! 'warning (string-append "LSP: orphan server " name
                                          " — not in the seeded catalog, run :lsp-uninstall to remove")))
              (else
               (lsp-install/register-server!
                 name
                 (path-join (lsp-install/server-dir name) (lsp-install/receipt-bin receipt))
                 (map (lambda (entry)
                        (cons (car entry) (path-join (lsp-install/server-dir name) (cdr entry))))
                      (lsp-install/receipt-env-dirs receipt)))))))
        (call! "stdlib/list-subdirs" sdir))))
  (lsp-install/set-default-lists!))
