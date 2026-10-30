;;; runtime/scheme/grammars.scm — core tree-sitter grammar registration.
;;; What this does and why: README.md, this directory.

;; ── Layout ────────────────────────────────────────────────────────────────────

;;; Sorted, and none for a path that is not a directory.
(define (%dir-entry-names dir)
  (if (is-dir? dir)
      (sort (map file-name (read-dir dir)) string<?)
      '()))

(define (grammars-dir)
  (path-join (data-dir) "grammars"))

(define (grammar-dir name)
  (path-join (grammars-dir) name))

(define (grammar-platform)
  (string-append (current-os!) "-" (target-arch!)))

(define (grammar-library-file name)
  (string-append name "." (platform-dll-extension!)))

(define (grammar-query-file file)
  (path-join "queries" file))

(define (grammar-library-path dir name)
  (path-join dir (grammar-library-file name)))

(define (%grammar-query-path dir file)
  (path-join dir (grammar-query-file file)))

(define (%grammar-highlights-path dir)
  (%grammar-query-path dir "highlights.scm"))

(define (%grammar-manifest-path dir)
  (path-join dir "manifest.scm"))

(define (%grammar-checked-path dir)
  (path-join dir "checked"))

;; ── Installs ──────────────────────────────────────────────────────────────────
;; See README.md.

(define (%grammar-install-name? entry)
  (let ((n (string->number entry)))
    (and (integer? n) (>= n 0) (equal? (number->string n) entry))))

;;; `name`'s install folder names, oldest first.
(define (grammar-installs name)
  (let ((dir (grammar-dir name)))
    (sort (filter (lambda (entry)
                    (and (%grammar-install-name? entry) (is-dir? (path-join dir entry))))
                  (%dir-entry-names dir))
          (lambda (a b) (< (string->number a) (string->number b))))))

;;; `name`'s current install: its newest install folder marked checked, or #f.
(define (grammar-installed-folder name)
  (let loop ((installs (reverse (grammar-installs name))))
    (and (pair? installs)
         (let ((folder (path-join (grammar-dir name) (car installs))))
           (if (path-exists? (%grammar-checked-path folder))
               folder
               (loop (cdr installs)))))))

;;; Marks the install folder `dir` as one whose grammar has loaded.
(define (mark-grammar-checked! dir)
  (close-output-port (open-output-file (%grammar-checked-path dir) #:exists 'truncate)))

(define (grammar-installed? name)
  (and (grammar-installed-folder name) #t))

;;; `(name . folder)` for each installed grammar of a language HUME knows, sorted by name,
;;; `folder` its current install.
(define (installed-grammar-folders)
  (let ((entries (%dir-entry-names (grammars-dir))))
    (if (null? entries)
        '()
        (let ((languages (bundled-languages)))
          (filter pair?
                  (map (lambda (name)
                         (let ((folder (and (member name languages)
                                            (grammar-installed-folder name))))
                           (and folder (cons name folder))))
                       entries))))))

;;; The parsed `manifest.scm` of the install folder `dir`.
(define (grammar-manifest dir)
  (read-data-file (%grammar-manifest-path dir)))

(define (grammar-manifest-ref manifest key)
  (let ((entry (and (list? manifest) (assoc key manifest))))
    (unless (pair? entry)
      (error (string-append "grammar manifest has no " (symbol->string key))))
    (cdr entry)))

;; ── Registration ──────────────────────────────────────────────────────────────
;; See README.md.

(define (%grammar-present-queries dir)
  (let loop ((kinds (grammar-optional-queries)) (present (hash)))
    (if (null? kinds)
        present
        (let ((path (%grammar-query-path dir (cdr (car kinds)))))
          (loop (cdr kinds)
                (if (path-exists? path)
                    (hash-insert present (car (car kinds)) path)
                    present))))))

(define (%grammar-symbol dir)
  (grammar-manifest-ref (grammar-manifest dir) 'symbol))

;;; Loads the grammar of the install folder `dir`, registering nothing.
(define (check-grammar! name dir)
  (%check-grammar! name (grammar-library-path dir name) (%grammar-symbol dir)
                   (%grammar-highlights-path dir) (%grammar-present-queries dir)))

;;; Registers the grammar of `name`'s install folder `folder`.
(define (register-installed-grammar! name folder)
  (%register-grammar! name (grammar-library-path folder name) (%grammar-symbol folder)
                      (%grammar-highlights-path folder) (%grammar-present-queries folder)))

(define (%register-installed-grammars!)
  (for-each
    (lambda (installed)
      (let ((name (car installed))
            (folder (cdr installed)))
        (with-handler
          (lambda (err)
            (log! 'warning (string-append
                            "grammar \"" name "\" is installed but broken (" (to-string err)
                            ") — run :plum-install-grammar " name " to repair")))
          (register-installed-grammar! name folder))))
    (installed-grammar-folders)))

(when (data-dir)
  (%register-installed-grammars!))
