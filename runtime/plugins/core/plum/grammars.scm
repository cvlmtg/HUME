;;; core:plum/grammars.scm — grammar INSTALL pipeline only. The source
;;; catalog, path helpers, and startup registration of already-compiled
;;; grammars live in core (runtime/scheme/grammars.scm) — see README.md.

(require "lib.scm")

;;; Helix commit pin, read once at plugin load.
(define *plum-helix-pin*
  (call-with-input-file (path-join (runtime-dir) "scheme" "helix-pin.scm") read))

(define (plum/helix-query-url name filename)
  (string-append "https://raw.githubusercontent.com/helix-editor/helix/"
                 *plum-helix-pin* "/runtime/queries/" name "/" filename))

;; ── Query inheritance resolution ──────────────────────────────────────────────
;; See README.md.

(define (plum/inherits-line? line)
  (starts-with? (trim line) "; inherits:"))

;;; Dependency names out of a `; inherits: a,b,c` line.
(define (plum/inherits-deps line)
  (let ((trimmed (trim line)))
    (map trim (split-many (trim (substring trimmed 11 (string-length trimmed))) ","))))

;;; `curl` is deliberately NOT wrapped in a `with-handler` — re-raising a
;;; native-builtin error from an inner handler into an outer one corrupts
;;; Steel 0.8.2's continuation stack. See README.md.
(define (plum/fetch-raw-query name filename)
  ;; `name` may come from an untrusted `; inherits:` line, unlike the
  ;; top-level grammar name — guard the scratch-file path.
  (unless (plum/safe-segment? name)
    (log! 'warn (string-append "plum/fetch-raw-query: rejecting unsafe grammar/dependency name \"" name "\""))
    (error (string-append "plum/fetch-raw-query: unsafe grammar/dependency name \"" name "\"")))
  (let ((tmp (path-join (grammar-sources-dir) (string-append "_fetch_" name "_" filename))))
    (run-inline-output! "curl" (list "-fsSL" "-o" tmp "--" (plum/helix-query-url name filename)))
    (let ((content (with-handler
                     (lambda (err) (call! "stdlib/delete-file" tmp) (raise-error err))
                     (plum/read-file tmp))))
      (call! "stdlib/delete-file" tmp)
      content)))

;;; Fully resolves any `; inherits:` chain into one string. `tolerant?`:
;;; a missing file resolves to `""` instead of raising — see README.md.
(define (plum/resolve-query name filename tolerant?)
  (let ((content (if tolerant?
                      (with-handler (lambda (err) #f) (plum/fetch-raw-query name filename))
                      (plum/fetch-raw-query name filename))))
    (if (not content)
        ""
        (let* ((lines (split-many content "\n"))
               (inherits-line (call! "stdlib/find" plum/inherits-line? lines)))
          (if inherits-line
              (string-join
                (append
                  (map (lambda (dep) (plum/resolve-query dep filename #t))
                       (plum/inherits-deps inherits-line))
                  (list (string-join (filter (lambda (l) (not (equal? l inherits-line))) lines) "\n")))
                "\n")
              content)))))

(define (plum/fetch-query! name filename dest)
  (call! "stdlib/write-file" dest (plum/resolve-query name filename #f)))

;; ── Injection dependencies ────────────────────────────────────────────────────
;; See README.md.

;;; Hash: name → (dep-name ...).
(define *plum-grammar-deps*
  (hash "markdown" (list "markdown.inline")))

(define (plum/grammar-deps name)
  (if (hash-contains? *plum-grammar-deps* name)
      (hash-ref *plum-grammar-deps* name)
      '()))

(define (plum/install-grammar-deps! name)
  (for-each
    (lambda (dep)
      (unless (grammar-installed? dep)
        (log! 'info (string-append "PLUM: installing dependency " dep " for " name))
        (plum/install-grammar dep)))
    (plum/grammar-deps name)))

;;; Tolerates a missing file — see README.md. Returns the path on success,
;;; `#f` if there's no such query to fetch.
(define (plum/try-fetch-query! name filename path-fn)
  (let ((path (path-fn name)))
    (with-handler
      (lambda (err)
        (log! 'trace (string-append "PLUM: no " filename " for " name " (" (to-string err) ")"))
        #f)
      (begin
        (plum/fetch-query! name filename path)
        path))))

(define (plum/try-fetch-injections! name)
  (plum/try-fetch-query! name "injections.scm" grammar-injections-path))

(define (plum/try-fetch-textobjects! name)
  (plum/try-fetch-query! name "textobjects.scm" grammar-textobjects-path))

;; ── Grammar discovery ─────────────────────────────────────────────────────────

(define (plum/not-installed? name)
  (not (grammar-installed? name)))

(define (plum/missing-grammars)
  (filter plum/not-installed? (grammar-source-names)))

(define (plum/orphan-grammars)
  (filter (lambda (name) (not (grammar-source-known? name)))
          (installed-grammars)))

;;; A string argument wins; otherwise falls back to the current buffer's
;;; language. Returns the name, or #f after reporting a status message.
(define (plum/resolve-grammar-arg cmd arg)
  (let ((name (call! "stdlib/resolve-lang-arg" cmd arg)))
    (cond ((not name) #f)
          ((not (grammar-source-known? name))
           (log! 'info (string-append cmd ": unknown grammar \"" name "\" — see :plum-list-grammars"))
           #f)
          (else name))))

;; ── Install pipeline ──────────────────────────────────────────────────────────

;;; Always from a clean slate — doubles as the repair path. See README.md
;;; for the numbered steps.
(define (plum/install-grammar name)
  (let* ((url     (grammar-source-url name))
         (rev     (grammar-source-rev name))
         (symbol  (grammar-source-symbol name))
         (subpath (grammar-source-subpath name))
         (src-dir (grammar-source-dir name))
         (build-dir (if (equal? subpath "")
                        src-dir
                        (path-join src-dir subpath)))
         (out-path (grammar-output-path name))
         (hl-path  (grammar-highlights-path name)))
    (plum/install-grammar-deps! name)
    ;; git clone refuses a non-empty dest — clear any stale source tree first.
    (call! "stdlib/delete-dir" src-dir)
    ;; Blobless clone (skip file-history blobs) at the pinned rev, then
    ;; checkout that exact revision.
    (run-inline-output! "git" (list "clone" "--filter=blob:none" "--" url src-dir))
    (run-inline-output! "git" (list "checkout" "--force" "--end-of-options" rev "--") #:cwd src-dir)
    (plum/fetch-query! name "highlights.scm" hl-path)
    ;; git prints its own progress; the C compiler stays silent until it's
    ;; done or errors, which on a slow grammar reads as a hang.
    (displayln (string-append "Compiling grammar for " name "..."))
    (compile-grammar! build-dir out-path)
    (register-grammar! name out-path symbol hl-path
                       (plum/try-fetch-injections! name)
                       (plum/try-fetch-textobjects! name))))

;; ── Commands ──────────────────────────────────────────────────────────────────

(define-typed-command! "plum-install-grammar"
  "Install (or repair) a tree-sitter grammar by name, always from a clean re-clone (default: the current buffer's language)."
  (lambda (arg)
    (let ((name (plum/resolve-grammar-arg "plum-install-grammar" arg)))
      (when name
        (log! 'info (string-append "PLUM: installing grammar for " name))
        (with-handler
          (lambda (err)
            (log! 'error (string-append "PLUM: install failed: " (to-string err))))
          (plum/install-grammar name)))))
  #:inline-output #t)

(define-command! "plum-ensure-grammars"
  "Install the named grammars (a list) that are not yet compiled."
  (lambda (grammars)
    (unless (and (list? grammars) (not (null? grammars)))
      (error "plum-ensure-grammars: requires a non-empty list of grammar names, e.g. (call! \"plum-ensure-grammars\" '(\"rust\" \"json\"))"))
    (let ((missing (filter plum/not-installed? grammars)))
      (if (null? missing)
          (log! 'info "PLUM: all requested grammars are installed")
          (plum/batch-run "installed grammar" missing plum/install-grammar))))
  #:inline-output #t)

(define-typed-command! "plum-list-grammars"
  "Log declared, installed, orphan, and missing grammar lists."
  (lambda ()
    (let ((declared  (grammar-source-names))
          (installed (installed-grammars))
          (orphans   (plum/orphan-grammars))
          (missing   (plum/missing-grammars)))
      (log! 'info (string-append "PLUM grammars declared:   " (string-join declared  ", ")))
      (log! 'info (string-append "PLUM grammars installed:  " (string-join installed ", ")))
      (log! 'info (string-append "PLUM grammars orphan:     " (string-join orphans   ", ")))
      (log! 'info (string-append "PLUM grammars missing:    " (string-join missing   ", "))))))

(define-typed-command! "plum-cleanup-grammars"
  "Delete compiled grammar files that are no longer in the declared source list."
  (lambda ()
    (let ((orphans (plum/orphan-grammars)))
      (if (null? orphans)
          (log! 'info "PLUM: no orphan grammars to remove")
          (plum/batch-run "removed grammar" orphans
            (lambda (name)
              (call! "stdlib/delete-file" (grammar-output-path name))))))))
