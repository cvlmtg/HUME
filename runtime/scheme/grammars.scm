;;; runtime/scheme/grammars.scm — core tree-sitter grammar registration.
;;;
;;; Loaded unconditionally at startup, after languages.scm and before
;;; init.scm (see scripting_setup.rs) — registers every already-compiled
;;; grammar found on disk. Passive: no subprocess, no network. Installing NEW
;;; grammars is core:plum's job (runtime/plugins/core/plum/grammars.scm);
;;; this file only makes already-installed ones take effect, so highlighting
;;; survives PLUM being absent from init.scm.

;; ── Grammar source registry ───────────────────────────────────────────────────

;;; Hash: name → (url rev symbol subpath)
(define *grammar-sources* (hash))

;;; Register a grammar source from a 5-tuple (name url rev symbol subpath).
(define (declare-grammar-source! entry)
  (let ((name    (list-ref entry 0))
        (url     (list-ref entry 1))
        (rev     (list-ref entry 2))
        (symbol  (list-ref entry 3))
        (subpath (list-ref entry 4)))
    (set! *grammar-sources*
          (hash-insert *grammar-sources* name (list url rev symbol subpath)))))

(for-each
  declare-grammar-source!
  (call-with-input-file
    (path-join (runtime-dir) "scheme" "grammar-sources.scm")
    read))

;;; Accessors.
(define (grammar-source-url name)
  (list-ref (hash-ref *grammar-sources* name) 0))
(define (grammar-source-rev name)
  (list-ref (hash-ref *grammar-sources* name) 1))
(define (grammar-source-symbol name)
  (list-ref (hash-ref *grammar-sources* name) 2))
(define (grammar-source-subpath name)
  (list-ref (hash-ref *grammar-sources* name) 3))

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

;;; Shared-library extension for compiled grammars on this platform. Mirrors
;;; the (removed) `grammar-output-path` Rust builtin's compile-time `cfg`
;;; dispatch as closely as a runtime string allows: `(hume-target)` only
;;; recognizes 4 platform strings and returns `#f` otherwise (unlike the
;;; Rust `cfg(target_os)` match, which covers every OS at compile time), so
;;; the `else` branch here — like the Rust `else` branch it replaces —
;;; defaults to "so" rather than erroring on an unrecognized platform.
(define (platform-grammar-ext)
  (let ((target (hume-target)))
    (cond ((and (string? target) (starts-with? target "darwin")) "dylib")
          ((and (string? target) (starts-with? target "windows")) "dll")
          (else "so"))))

;;; Path a compiled grammar for `name` lives (or will live) at.
(define (grammar-output-path name)
  (path-join (grammars-dir) (string-append name "." (platform-grammar-ext))))

;;; #t if the compiled grammar for `name` exists on disk.
(define (grammar-installed? name)
  (path-exists? (grammar-output-path name)))

;; ── Startup registration ──────────────────────────────────────────────────────

;;; Passive: registers already-compiled grammars only, no subprocess. See
;;; core:plum/README.md for the install pipeline that produces these files.
(define (register-installed-grammars!)
  (for-each
    (lambda (name)
      (let ((out  (grammar-output-path name))
            (hl   (grammar-highlights-path name))
            (inj  (grammar-injections-path name))
            (sym  (grammar-source-symbol name)))
        (when (and (grammar-installed? name) (path-exists? hl))
          (register-grammar! name out sym hl (if (path-exists? inj) inj #f)))))
    (hash-keys->list *grammar-sources*)))

;;; `data-dir` is `#f` when HOME/APPDATA is unset — nothing to scan.
(when (data-dir)
  (register-installed-grammars!))
