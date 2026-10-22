;;; core:lsp-install/lock.scm — see README.md.

(require-builtin steel/time)
(require "receipts.scm")

(provide lsp-install/with-lock!)

(define lsp-install/lock-stale-seconds 3600)

(define (lsp-install/lock-path)
  (path-join (lsp-install/servers-dir) ".install-lock"))

(define (lsp-install/create-lock! path)
  (with-handler (lambda (err) #f)
    (let ((port (open-output-file path)))
      (write-string (number->string (current-process-id)) port)
      (close-output-port port)
      #t)))

;;; See docs/servers.md's "Install lock".
(define (lsp-install/lock-owner path)
  (with-handler (lambda (err) #f)
    (let ((pid (string->number (trim (call-with-input-file path read-port-to-string)))))
      (and (integer? pid) (> pid 0) pid))))

(define (lsp-install/lock-stale? path)
  (with-handler (lambda (err) #f)
    (> (duration->seconds
         (system-time-duration-since (system-time/now)
                                     (fs-metadata-modified (file-metadata path))))
       lsp-install/lock-stale-seconds)))

(define (lsp-install/acquire-lock!)
  (let ((path (lsp-install/lock-path)))
    (create-directory! (lsp-install/servers-dir))
    (unless (lsp-install/create-lock! path)
      (unless (path-exists? path)
        (error (string-append "lsp-install/acquire-lock!: cannot create lock at " path)))
      (let* ((owner (lsp-install/lock-owner path))
             (exited? (and owner (not (process-alive? owner)))))
        (unless (or exited? (lsp-install/lock-stale? path))
          (error "lsp-install/acquire-lock!: another install/uninstall is already in progress"))
        (log! 'warn
          (if exited?
              (string-append "LSP: install lock left by process " (number->string owner)
                             ", which has exited, replacing")
              (string-append "LSP: stale install lock (older than "
                             (number->string (quotient lsp-install/lock-stale-seconds 3600))
                             "h), replacing"))))
      (call! "stdlib/delete-file!" path)
      (unless (lsp-install/create-lock! path)
        (error (string-append "lsp-install/acquire-lock!: cannot create lock after removing stale one at "
                              path))))))

(define (lsp-install/release-lock!)
  (call! "stdlib/delete-file!" (lsp-install/lock-path)))

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
