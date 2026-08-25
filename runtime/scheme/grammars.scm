;;; runtime/scheme/grammars.scm — core tree-sitter grammar registration.
;;; What this does and why: README.md, this directory.

;; ── Grammar source catalog ────────────────────────────────────────────────────

(define *grammar-sources-cache* (box #f))

(define (read-grammar-sources)
  (let loop ((entries (call-with-input-file
                        (path-join (runtime-dir) "scheme" "grammar-sources.scm")
                        read))
             (acc (hash)))
    (if (null? entries)
        acc
        (let ((entry (car entries)))
          (loop (cdr entries)
                (hash-insert acc
                             (list-ref entry 0)
                             (list (list-ref entry 1) (list-ref entry 2)
                                   (list-ref entry 3) (list-ref entry 4))))))))

(define (grammar-sources)
  (unless (unbox *grammar-sources-cache*)
    (set-box! *grammar-sources-cache* (read-grammar-sources)))
  (unbox *grammar-sources-cache*))

(define (grammar-source-names)
  (hash-keys->list (grammar-sources)))
(define (grammar-source-known? name)
  (hash-contains? (grammar-sources) name))
(define (grammar-source-url name)
  (list-ref (hash-ref (grammar-sources) name) 0))
(define (grammar-source-rev name)
  (list-ref (hash-ref (grammar-sources) name) 1))
(define (grammar-source-symbol name)
  (list-ref (hash-ref (grammar-sources) name) 2))
(define (grammar-source-subpath name)
  (list-ref (hash-ref (grammar-sources) name) 3))

;; ── Path helpers ──────────────────────────────────────────────────────────────

(define (grammars-dir)
  (path-join (data-dir) "grammars"))

(define (grammar-sources-dir)
  (path-join (grammars-dir) "sources"))

(define (grammar-source-dir name)
  (path-join (grammar-sources-dir) name))

(define (grammar-highlights-path name)
  (path-join (grammar-source-dir name) "highlights.scm"))

(define (grammar-injections-path name)
  (path-join (grammar-source-dir name) "injections.scm"))

(define (platform-grammar-ext)
  (let ((target (hume-target)))
    (cond ((and (string? target) (starts-with? target "darwin")) "dylib")
          ((and (string? target) (starts-with? target "windows")) "dll")
          (else "so"))))

(define (grammar-output-path name)
  (path-join (grammars-dir) (string-append name "." (platform-grammar-ext))))

(define (grammar-installed? name)
  (path-exists? (grammar-output-path name)))

;; ── Installed-grammar discovery ───────────────────────────────────────────────

(define (installed-grammars)
  (let ((gdir (grammars-dir)))
    (if (not (path-exists? gdir))
        '()
        (let* ((suffix (string-append "." (platform-grammar-ext)))
               (slen   (string-length suffix)))
          (map (lambda (f) (substring f 0 (- (string-length f) slen)))
               (filter (lambda (f) (and (ends-with? f suffix)
                                        (> (string-length f) slen)))
                       (sort (map file-name (read-dir gdir)) string<?)))))))

;; ── Startup registration ──────────────────────────────────────────────────────

(define (register-installed-grammars!)
  (for-each
    (lambda (name)
      (when (grammar-source-known? name)
        (let ((hl (grammar-highlights-path name)))
          (if (not (path-exists? hl))
              (log! 'warn (string-append
                            "grammar \"" name "\" is compiled but missing its highlights "
                            "query — run :plum-install-grammar " name " to repair"))
              (let ((inj (grammar-injections-path name)))
                (register-grammar! name
                                   (grammar-output-path name)
                                   (grammar-source-symbol name)
                                   hl
                                   (if (path-exists? inj) inj #f)))))))
    (installed-grammars)))

(when (data-dir)
  (register-installed-grammars!))
