;;; core:lsp-install/catalog.scm — see README.md.

(provide lsp-install/ref
         lsp-install/ref-or
         lsp-install/data-path
         lsp-install/read-record
         lsp-install/registration
         lsp-install/seeded-server?
         lsp-install/server-names
         lsp-install/language-list
         lsp-install/seeded-language?
         lsp-install/language-names
         lsp-install/requirement
         lsp-install/server-languages
         lsp-install/primary)

(define lsp-install/dir (plugin-dir))

(define (lsp-install/data-path kind)
  (path-join lsp-install/dir "data" kind))

;; `name`'s record in data directory `kind`, or #f when it has no file there.
(define (lsp-install/read-record kind name)
  (let ((path (data-file (lsp-install/data-path kind) name)))
    (and path (read-data-file path))))

(define (lsp-install/ref fields key)
  (cdr (assoc key fields)))

;; A field absent from the record is `default`.
(define (lsp-install/ref-or fields key default)
  (let ((entry (assoc key fields)))
    (if entry (cdr entry) default)))

(define (lsp-install/registration name)
  (lsp-install/read-record "registrations" name))

(define (lsp-install/seeded-server? name)
  (if (data-file (lsp-install/data-path "registrations") name) #t #f))

(define (lsp-install/server-names)
  (data-file-names (lsp-install/data-path "registrations")))

(define lsp-install/language-rows
  (read-data-file (path-join lsp-install/dir "data" "language-servers.scm")))

(define lsp-install/language-lists
  (let loop ((rows lsp-install/language-rows) (table (hash)))
    (if (null? rows)
        table
        (loop (cdr rows)
              (hash-insert table (car (car rows)) (lsp-install/ref (cdr (car rows)) 'servers))))))

;; A language's server entries, or #f.
(define (lsp-install/language-list lang)
  (and (hash-contains? lsp-install/language-lists lang)
       (hash-ref lsp-install/language-lists lang)))

(define (lsp-install/seeded-language? lang)
  (hash-contains? lsp-install/language-lists lang))

(define (lsp-install/language-names)
  (map car lsp-install/language-rows))

(define (lsp-install/requirement name)
  (lsp-install/read-record "requirements" name))

;; The server `:lsp-install <lang>` installs for a language: its first, or #f.
(define (lsp-install/primary lang)
  (let ((entries (lsp-install/language-list lang)))
    (and entries (car (car entries)))))

;; Every language a server is listed for, in name order.
(define (lsp-install/server-languages name)
  (filter (lambda (lang) (member name (map car (lsp-install/language-list lang))))
          (lsp-install/language-names)))
