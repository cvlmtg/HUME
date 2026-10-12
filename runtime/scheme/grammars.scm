;;; runtime/scheme/grammars.scm — core tree-sitter grammar registration.
;;; What this does and why: README.md, this directory.

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

(define (grammar-textobjects-path name)
  (path-join (grammar-source-dir name) "textobjects.scm"))

(define (platform-grammar-ext)
  (let ((os (current-os!)))
    (cond ((equal? os "macos") "dylib")
          ((equal? os "windows") "dll")
          (else "so"))))

(define (grammar-output-path name)
  (path-join (grammars-dir) (string-append name "." (platform-grammar-ext))))

(define (grammar-installed? name)
  (path-exists? (grammar-output-path name)))

;; ── Bundled grammar fields ────────────────────────────────────────────────────

(define (grammar-url grammar) (list-ref grammar 0))
(define (grammar-rev grammar) (list-ref grammar 1))
(define (grammar-symbol grammar) (list-ref grammar 2))
(define (grammar-subpath grammar) (list-ref grammar 3))

;; ── Installed-grammar discovery ───────────────────────────────────────────────

(define (installed-grammars)
  (stems-with-suffix (grammars-dir) (string-append "." (platform-grammar-ext))))

;; ── Startup registration ──────────────────────────────────────────────────────

(define (register-installed-grammars!)
  (for-each
    (lambda (name)
      (let ((source (bundled-language-grammar name)))
        (when source
          (let ((hl (grammar-highlights-path name)))
            (if (not (path-exists? hl))
                (log! 'warn (string-append
                              "grammar \"" name "\" is compiled but missing its highlights "
                              "query — run :plum-install-grammar " name " to repair"))
                (let ((inj (grammar-injections-path name))
                      (to (grammar-textobjects-path name)))
                  (register-grammar! name
                                     (grammar-output-path name)
                                     (grammar-symbol source)
                                     hl
                                     #:injections (if (path-exists? inj) inj #f)
                                     #:textobjects (if (path-exists? to) to #f))))))))
    (installed-grammars)))

(when (data-dir)
  (register-installed-grammars!))
