;;; core:lsp-install/catalog.scm — see README.md.

(provide lsp-install/field
         lsp-install/servers-catalog
         lsp-install/sources-catalog
         lsp-install/lang->server)

(define lsp-install/dir (plugin-dir))

(define (lsp-install/read-data file)
  (call-with-input-file (path-join lsp-install/dir file) read))

(define (lsp-install/index-entries entries)
  (let loop ((entries entries) (index (hash)))
    (if (null? entries)
        index
        (loop (cdr entries) (hash-insert index (car (car entries)) (cdr (car entries)))))))

(define *lsp-install-servers* (lsp-install/index-entries (lsp-install/read-data "servers.scm")))
(define *lsp-install-sources* (lsp-install/index-entries (lsp-install/read-data "sources.scm")))

(define (lsp-install/servers-catalog) *lsp-install-servers*)
(define (lsp-install/sources-catalog) *lsp-install-sources*)

(define (lsp-install/field fields key)
  (cond ((null? fields) #f)
        ((equal? (car (car fields)) key) (car fields))
        (else (lsp-install/field (cdr fields) key))))

(define *lsp-install-lang->server*
  (let loop ((names (hash-keys->list *lsp-install-servers*)) (index (hash)))
    (if (null? names)
        index
        (loop (cdr names)
              (let ((langs (cdr (lsp-install/field (hash-ref *lsp-install-servers* (car names))
                                                   'languages))))
                (let inner ((langs langs) (index index))
                  (if (null? langs)
                      index
                      (inner (cdr langs)
                             (hash-insert index (car (car langs)) (car names))))))))))

(define (lsp-install/lang->server) *lsp-install-lang->server*)
