;;; core:lsp-install/blocker.scm — see README.md.

(require "catalog.scm")
(require "platform.scm")

(provide lsp-install/target-row lsp-install/row-fmt lsp-install/row-tools
         lsp-install/install-blocker)

(define (lsp-install/row-fmt row) (cadr row))
(define (lsp-install/row-tools row) (cddr row))

(define (lsp-install/row-covers? row target)
  (or (equal? (car row) '*) (member target (car row))))

;; The requirements row `name` has for this platform, or #f.
(define (lsp-install/target-row name)
  (let ((fields (lsp-install/requirement name)))
    (and fields lsp-install/target (assoc 'targets fields)
         (let ((want (string->symbol lsp-install/target)))
           (call! "stdlib/find" (lambda (row) (lsp-install/row-covers? row want))
                  (lsp-install/ref fields 'targets))))))

(define (lsp-install/missing-tool row)
  (call! "stdlib/find" (lambda (tool) (not (which tool))) (lsp-install/row-tools row)))

;; A string naming what blocks installing `name` here, or #f.
(define (lsp-install/install-blocker name)
  (let ((fields (lsp-install/requirement name)))
    (cond
      ((not lsp-install/target) "unsupported platform")
      ((not fields) "no install source")
      ((assoc 'blocked fields)
       (string-append "not installable (kind " (symbol->string (lsp-install/ref fields 'blocked))
                      ") in v1"))
      (else
       (let ((row (lsp-install/target-row name)))
         (cond
           ((not row)
            (if (equal? (lsp-install/ref fields 'missing) 'download)
                "no prebuilt asset for this platform"
                "not supported on this platform"))
           ((lsp-install/missing-tool row)
            => (lambda (tool)
                 (string-append "requires '" tool "' on $PATH, which was not found")))
           (else #f)))))))
