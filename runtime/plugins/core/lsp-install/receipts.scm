;;; core:lsp-install/receipts.scm — see README.md.

(require "catalog.scm")

(provide lsp-install/servers-dir lsp-install/server-dir lsp-install/receipt-path
         lsp-install/read-receipt lsp-install/write-receipt!
         lsp-install/receipt-bin lsp-install/receipt-version lsp-install/receipt-env-dirs)

(define (lsp-install/servers-dir) (path-join (data-dir) "servers"))
(define (lsp-install/server-dir name) (path-join (lsp-install/servers-dir) name))
(define (lsp-install/receipt-path name) (path-join (lsp-install/server-dir name) "receipt.scm"))

(define (lsp-install/read-receipt name)
  (with-handler (lambda (err) #f)
    (call-with-input-file (lsp-install/receipt-path name) read)))

(define (lsp-install/receipt-bin receipt) (cdr (lsp-install/field receipt 'bin)))
(define (lsp-install/receipt-version receipt) (cdr (lsp-install/field receipt 'version)))

(define (lsp-install/receipt-env-dirs receipt)
  (let ((field (lsp-install/field receipt 'env-dirs)))
    (if field (cdr field) '())))

(define (lsp-install/scheme-quote s)
  (string-append "\"" (string-replace (string-replace s "\\" "\\\\") "\"" "\\\"") "\""))

(define (lsp-install/write-receipt! name version bin env-dirs)
  (call! "stdlib/write-file!" (lsp-install/receipt-path name)
    (string-append "((name . " (lsp-install/scheme-quote name) ")"
                   " (version . " (lsp-install/scheme-quote version) ")"
                   " (bin . " (lsp-install/scheme-quote bin) ")"
                   " (env-dirs"
                   (apply string-append
                          (map (lambda (entry)
                                 (string-append " (" (lsp-install/scheme-quote (car entry))
                                                " . " (lsp-install/scheme-quote (cdr entry)) ")"))
                               env-dirs))
                   "))")))
