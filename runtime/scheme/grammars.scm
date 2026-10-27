;;; runtime/scheme/grammars.scm — core tree-sitter grammar registration.
;;; What this does and why: README.md, this directory.

;; ── Path helpers ──────────────────────────────────────────────────────────────

(define (grammars-dir)
  (path-join (data-dir) "grammars"))

(define (grammar-sources-dir)
  (path-join (grammars-dir) "sources"))

(define (grammar-source-dir name)
  (path-join (grammar-sources-dir) name))

(define (grammar-query-path name file)
  (path-join (grammar-source-dir name) file))

(define (grammar-highlights-path name)
  (grammar-query-path name "highlights.scm"))

(define (platform-grammar-ext)
  (let ((os (current-os!)))
    (cond ((equal? os "macos") "dylib")
          ((equal? os "windows") "dll")
          (else "so"))))

(define (grammar-output-path name)
  (path-join (grammars-dir) (string-append name "." (platform-grammar-ext))))

(define (grammar-installed? name)
  (path-exists? (grammar-output-path name)))

;; ── Installed-grammar discovery ───────────────────────────────────────────────

(define (installed-grammars)
  (stems-with-suffix (grammars-dir) (string-append "." (platform-grammar-ext))))

;; ── Registration ──────────────────────────────────────────────────────────────
;; See README.md.

(define (grammar-present-queries name)
  (let loop ((kinds (grammar-optional-queries)) (present (hash)))
    (if (null? kinds)
        present
        (let ((path (grammar-query-path name (cdr (car kinds)))))
          (loop (cdr kinds)
                (if (path-exists? path)
                    (hash-insert present (car (car kinds)) path)
                    present))))))

(define (register-grammar-from-sources! name grammar-path symbol)
  (%register-grammar! name grammar-path symbol
                      (grammar-highlights-path name)
                      (grammar-present-queries name)))

(define (register-installed-grammars!)
  (for-each
    (lambda (name)
      (let ((source (bundled-language-grammar name)))
        (when source
          (let ((hl (grammar-highlights-path name)))
            (if (not (path-exists? hl))
                (log! 'warning (string-append
                              "grammar \"" name "\" is compiled but missing its highlights "
                              "query — run :plum-install-grammar " name " to repair"))
                (register-grammar-from-sources! name
                                                (grammar-output-path name)
                                                (hash-ref source 'symbol)))))))
    (installed-grammars)))

(when (data-dir)
  (register-installed-grammars!))
