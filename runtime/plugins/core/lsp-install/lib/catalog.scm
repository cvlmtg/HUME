;;; core:lsp-install/catalog.scm — see README.md.

(provide lsp-install/ref
         lsp-install/ref-or
         lsp-install/require-stdlib!
         lsp-install/read-data
         lsp-install/index-entries
         lsp-install/servers
         lsp-install/lookup
         lsp-install/requirement
         lsp-install/language-lists
         lsp-install/server-languages
         lsp-install/primary)

(define lsp-install/dir (plugin-dir))

(define (lsp-install/require-stdlib!)
  (unless (member "core:stdlib" (declared-plugins))
    (error "core:lsp-install: requires core:stdlib — add (load-plugin! \"core:stdlib\") before (load-plugin! \"core:lsp-install\")")))

(define (lsp-install/read-data file)
  (call-with-input-file (path-join lsp-install/dir "data" file) read))

(define (lsp-install/index-entries entries)
  (let loop ((entries entries) (index (hash)))
    (if (null? entries)
        index
        (loop (cdr entries) (hash-insert index (car (car entries)) (cdr (car entries)))))))

(define lsp-install/servers (lsp-install/index-entries (lsp-install/read-data "servers.scm")))
(define lsp-install/requirements #f)

(define (lsp-install/ref fields key)
  (cdr (assoc key fields)))

;; A field absent from the tail is `default`.
(define (lsp-install/ref-or fields key default)
  (let ((entry (assoc key fields)))
    (if entry (cdr entry) default)))

(define (lsp-install/lookup table name)
  (if (hash-contains? table name)
      (hash-ref table name)
      #f))

(define (lsp-install/requirement name)
  (unless lsp-install/requirements
    (set! lsp-install/requirements
          (lsp-install/index-entries (lsp-install/read-data "requirements.scm"))))
  (lsp-install/lookup lsp-install/requirements name))

(define lsp-install/language-rows (lsp-install/read-data "language-servers.scm"))

;; language -> `((servers entry ...))`. Each server entry is
;; `("name")` or `("name" (only-features "f" ...))` / `("name" (except-features "f" ...))`.
(define lsp-install/language-lists (lsp-install/index-entries lsp-install/language-rows))

;; The server `:lsp-install <lang>` installs for a language: its first, or #f.
(define (lsp-install/primary lang)
  (let ((fields (lsp-install/lookup lsp-install/language-lists lang)))
    (and fields (car (car (lsp-install/ref fields 'servers))))))

;; Every language a server is listed for, in catalog order.
(define (lsp-install/server-languages name)
  (map car
       (filter (lambda (row)
                 (member name (map car (lsp-install/ref (cdr row) 'servers))))
               lsp-install/language-rows)))
