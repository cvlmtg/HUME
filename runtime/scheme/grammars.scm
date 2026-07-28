;;; runtime/scheme/grammars.scm — core tree-sitter grammar registration.
;;;
;;; Loaded unconditionally at startup, after languages.scm and before
;;; init.scm (see scripting_setup.rs) — registers every already-compiled
;;; grammar found on disk. Passive: no subprocess, no network. Installing NEW
;;; grammars is core:plum's job (runtime/plugins/core/plum/grammars.scm);
;;; this file only makes already-installed ones take effect, so highlighting
;;; survives PLUM being absent from init.scm.
;;;
;;; Registration is driven by the install directory, not by the source
;;; catalog: `<data>/grammars/` holds one compiled file per installed grammar
;;; and does not exist at all until something is installed, so a setup with no
;;; grammars settles the whole question in one `path-exists?` instead of
;;; probing 350+ catalog entries that cannot match.

;; ── Grammar source catalog ────────────────────────────────────────────────────

;;; Hash: name → (url rev symbol subpath), parsed on first use.
;;;
;;; Lazy because nothing needs it until a grammar is actually installed or a
;;; `:plum-*-grammar` command runs — and reading + parsing the catalog's 350+
;;; 5-tuples is a measurable slice of startup that a fresh setup would pay for
;;; nothing. A `box` rather than `set!` on a plain global: core:plum reaches
;;; these bindings from inside a `require`d module, and `box` is the pattern
;;; already proven across that boundary (see `debounce` in builtins/bootstrap.scm).
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

;;; Accessors. Every one forces the catalog.
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

;;; Shared-library extension for compiled grammars on this platform.
;;; `(hume-target)` recognizes 4 platform strings and returns `#f` for
;;; anything else, so the `else` branch defaults to "so" rather than
;;; erroring on an unrecognized platform.
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

;; ── Installed-grammar discovery ───────────────────────────────────────────────

;;; Names of the compiled grammars in <data>/grammars/ that this platform can
;;; load. `read-dir` already proves each file exists, so matching the platform
;;; extension here does the whole job a second `stat` would: a `.so` left behind
;;; on macOS is dropped before it can reach dlopen, and the `sources/`
;;; subdirectory and dotfiles fall out of the same test for free.
;;; `'()` when nothing has ever been installed — the directory is created by
;;; the first install, so its absence is the common fresh-setup case.
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

;;; Passive: registers already-compiled grammars only, no subprocess. See
;;; core:plum/README.md for the install pipeline that produces these files.
;;;
;;; `grammar-source-known?` is what makes walking the directory safe: an
;;; orphan file (installed, then dropped from the catalog by a HUME update)
;;; has no tree-sitter symbol to look up.
(define (register-installed-grammars!)
  (for-each
    (lambda (name)
      (let ((hl (grammar-highlights-path name)))
        (when (and (grammar-source-known? name)
                   (path-exists? hl))
          (let ((inj (grammar-injections-path name)))
            (register-grammar! name
                               (grammar-output-path name)
                               (grammar-source-symbol name)
                               hl
                               (if (path-exists? inj) inj #f))))))
    (installed-grammars)))

;;; `data-dir` is `#f` when HOME/APPDATA is unset — nothing to scan.
(when (data-dir)
  (register-installed-grammars!))
