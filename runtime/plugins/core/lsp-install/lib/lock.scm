;;; core:lsp-install/lock.scm — see README.md.

(require "receipts.scm")

(provide lsp-install/with-lock!)

;;; See docs/servers.md's "Install lock".
(define (lsp-install/lock-path)
  (path-join (lsp-install/servers-dir) ".install-lock"))

(define (lsp-install/acquire-lock!)
  (create-directory! (lsp-install/servers-dir))
  (unless (try-lock-file! (lsp-install/lock-path))
    (error "lsp-install/acquire-lock!: another install/uninstall is already in progress")))

(define (lsp-install/release-lock!)
  (unlock-file! (lsp-install/lock-path)))

(define (lsp-install/with-lock! what thunk)
  (let ((acquired?
          (with-handler
            (lambda (err) (log! 'error (string-append "LSP: " (to-string err))) #f)
            (begin (lsp-install/acquire-lock!) #t))))
    (and acquired?
         (with-handler
           (lambda (err)
             (lsp-install/release-lock!)
             (log! 'error (string-append "LSP: " what " failed: " (to-string err)))
             #f)
           (begin (thunk) (lsp-install/release-lock!) #t)))))
