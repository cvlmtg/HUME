;;; core:lsp-install/receipts.scm — see README.md.

(require "catalog.scm")

(provide lsp-install/servers-dir lsp-install/server-dir
         lsp-install/current-install lsp-install/build-new-install!
         lsp-install/read-receipt
         lsp-install/receipt-bin lsp-install/receipt-version lsp-install/receipt-env-dirs)

(define (lsp-install/servers-dir) (path-join (data-dir) "servers"))
(define (lsp-install/server-dir name) (path-join (lsp-install/servers-dir) name))
(define (lsp-install/receipt-path folder) (path-join folder "receipt.scm"))

;; Install folders: see docs/architecture.md's "Install folders and receipts".

(define (lsp-install/install-number entry)
  (let ((n (string->number entry)))
    (and (integer? n) (>= n 0) (equal? (number->string n) entry) n)))

;;; `name`'s install numbers, oldest first.
(define (lsp-install/installs name)
  (let ((dir (lsp-install/server-dir name)))
    (if (path-exists? dir)
        (sort (filter number? (map lsp-install/install-number (call! "stdlib/list-entries" dir)))
              <)
        '())))

;;; `(folder . receipt)` for `name`'s current install, its newest install folder with a
;;; readable receipt, or #f.
(define (lsp-install/current-install name)
  (let loop ((installs (reverse (lsp-install/installs name))))
    (and (pair? installs)
         (let* ((folder  (path-join (lsp-install/server-dir name) (number->string (car installs))))
                (receipt (with-handler (lambda (err) #f)
                           (read-data-file (lsp-install/receipt-path folder)))))
           (if receipt
               (cons folder receipt)
               (loop (cdr installs)))))))

;;; A path for `name`'s next install folder, numbered after every other.
(define (lsp-install/new-install-folder name)
  (let ((installs (lsp-install/installs name)))
    (path-join (lsp-install/server-dir name)
               (number->string
                 (if (null? installs) 1 (+ 1 (last installs)))))))

(define (lsp-install/read-receipt name)
  (let ((current (lsp-install/current-install name)))
    (and current (cdr current))))

(define (lsp-install/receipt-bin receipt) (lsp-install/ref receipt 'bin))
(define (lsp-install/receipt-version receipt) (lsp-install/ref receipt 'version))

(define (lsp-install/receipt-env-dirs receipt)
  (let ((field (assoc 'env-dirs receipt)))
    (if field (cdr field) '())))

(define (lsp-install/scheme-quote s)
  (string-append "\"" (string-replace (string-replace s "\\" "\\\\") "\"" "\\\"") "\""))

(define (lsp-install/write-receipt! folder name version bin env-dirs)
  (call! "stdlib/write-file!" (lsp-install/receipt-path folder)
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

;;; Builds `name`'s next install with `(build folder)`, which answers `(bin . env-dirs)`,
;;; commits it under `version` and answers the installed binary's path.
(define (lsp-install/build-new-install! name version build)
  (let* ((dir       (lsp-install/server-dir name))
         (folder    (lsp-install/new-install-folder name))
         (installed
           (with-handler
             (lambda (err)
               (with-handler
                 (lambda (delete-err)
                   (log! 'error (string-append "LSP: could not delete the failed install in "
                                               folder ": " (to-string delete-err))))
                 (call! "stdlib/delete-dir!" folder)
                 (when (and (path-exists? dir) (null? (call! "stdlib/list-entries" dir)))
                   (remove-dir! dir)))
               (raise-error err))
             (create-directory! folder)
             (let ((installed (build folder)))
               (lsp-install/write-receipt! folder name version (car installed) (cdr installed))
               installed))))
    (for-each
      (lambda (entry)
        (let ((path (path-join dir entry)))
          (unless (equal? path folder)
            (with-handler
              (lambda (err)
                (log! 'warning (string-append "LSP: " name " is installed, but " path
                                              " could not be deleted, the next :lsp-install of "
                                              name " tries again: " (to-string err))))
              (if (is-dir? path)
                  (remove-dir! path)
                  (delete-file! path))))))
      (call! "stdlib/list-entries" dir))
    (path-join folder (car installed))))
