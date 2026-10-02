;;; core:lsp-install/catalog.scm — see README.md.

(provide lsp-install/ref
         lsp-install/require-stdlib!
         lsp-install/read-data
         lsp-install/index-entries
         lsp-install/servers
         lsp-install/requirement
         lsp-install/lang->server)

(define lsp-install/dir (plugin-dir))

(define (lsp-install/require-stdlib!)
  (unless (member "core:stdlib" (declared-plugins))
    (error "core:lsp-install: requires core:stdlib — add (load-plugin! \"core:stdlib\") before (load-plugin! \"core:lsp-install\")")))

(define (lsp-install/read-data file)
  (call-with-input-file (path-join lsp-install/dir file) read))

(define (lsp-install/index-entries entries)
  (let loop ((entries entries) (index (hash)))
    (if (null? entries)
        index
        (loop (cdr entries) (hash-insert index (car (car entries)) (cdr (car entries)))))))

(define lsp-install/servers (lsp-install/index-entries (lsp-install/read-data "servers.scm")))
(define lsp-install/requirements (lsp-install/index-entries (lsp-install/read-data "requirements.scm")))

(define (lsp-install/ref fields key)
  (cdr (assoc key fields)))

(define (lsp-install/requirement name)
  (if (hash-contains? lsp-install/requirements name)
      (hash-ref lsp-install/requirements name)
      #f))

(define lsp-install/lang->server
  (let loop ((names (hash-keys->list lsp-install/servers)) (index (hash)))
    (if (null? names)
        index
        (loop (cdr names)
              (let inner ((langs (lsp-install/ref (hash-ref lsp-install/servers (car names))
                                                  'languages))
                          (index index))
                (if (null? langs)
                    index
                    (inner (cdr langs)
                           (hash-insert index (car (car langs)) (car names)))))))))
