;;; runtime/scheme/grammars.scm — core tree-sitter grammar registration.
;;; What this does and why: README.md, this directory.

;; ── Layout ────────────────────────────────────────────────────────────────────

;;; Sorted, and none for a path that is not a directory.
(define (dir-entry-names dir)
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
                  (dir-entry-names dir))
          (lambda (a b) (< (string->number a) (string->number b))))))

;;; `name`'s current install folder, or #f when it has none.
(define (%grammar-current-install name)
  (let ((installs (grammar-installs name)))
    (and (pair? installs)
         (path-join (grammar-dir name) (last installs)))))

;;; `name`'s current install folder when it holds this platform's library, else #f.
(define (grammar-installed-folder name)
  (let ((dir (%grammar-current-install name)))
    (and dir (path-exists? (grammar-library-path dir name)) dir)))

(define (grammar-installed? name)
  (and (grammar-installed-folder name) #t))

;;; `(name . folder)` for each installed grammar of a language HUME knows, sorted by name,
;;; `folder` its current install.
(define (installed-grammar-folders)
  (let ((languages (bundled-languages)))
    (filter pair?
            (map (lambda (name)
                   (let ((folder (and (member name languages) (grammar-installed-folder name))))
                     (and folder (cons name folder))))
                 (dir-entry-names (grammars-dir))))))

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
  (grammar-manifest-ref (read-data-file (%grammar-manifest-path dir)) 'symbol))

;;; Loads `dir`'s grammar with its library at `library`, registering nothing.
(define (check-grammar! name dir library)
  (%check-grammar! name library (%grammar-symbol dir) (%grammar-highlights-path dir)
                   (%grammar-present-queries dir)))

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
          (unless (path-exists? (%grammar-highlights-path folder))
            (error "its highlights query is missing"))
          (register-installed-grammar! name folder))))
    (installed-grammar-folders)))

(when (data-dir)
  (%register-installed-grammars!))
