;;; core:plum/grammars.scm — the grammar install pipeline; see README.md.

(require "lib/helpers.scm")

(plum/require-stdlib!)

;;; Helix commit pin, read once at plugin load.
(define *plum-helix-pin*
  (read-data-file (path-join (runtime-dir) "scheme" "helix-pin.scm")))

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

;;; See README.md's "Query fetching".
(define (plum/fetch-raw-query name filename)
  ;; `name` may be untrusted (an `; inherits:` line).
  (unless (safe-path-segment? name)
    (log! 'warning (string-append "plum/fetch-raw-query: rejecting unsafe grammar/dependency name \"" name "\""))
    (error (string-append "plum/fetch-raw-query: unsafe grammar/dependency name \"" name "\"")))
  (let ((tmp (path-join (plum/staging-dir) (string-append "_fetch_" name "_" filename))))
    (create-directory! (plum/staging-dir))
    (with-handler
      (lambda (err)
        (call! "stdlib/delete-file!" tmp)
        (error (string-append "plum/fetch-raw-query: cannot fetch " filename " for " name ": "
                              (to-string err))))
      (run-inline-output! "curl" (list "-fsSL" "-o" tmp "--" (plum/helix-query-url name filename)))
      (let ((content (call-with-input-file tmp read-port-to-string)))
        (call! "stdlib/delete-file!" tmp)
        content))))

;;; Fully resolves any `; inherits:` chain into one string — see README.md.
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
  (call! "stdlib/write-file!" dest (plum/resolve-query name filename #f)))

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

;;; Tolerates a missing file — see README.md.
(define (plum/try-fetch-query! name filename dir)
  (with-handler
    (lambda (err)
      (log! 'trace (string-append "PLUM: no " filename " for " name " (" (to-string err) ")")))
    (plum/fetch-query! name filename (path-join dir filename))))

;; ── Grammar discovery ─────────────────────────────────────────────────────────

(define (plum/not-installed? name)
  (not (grammar-installed? name)))

(define (plum/no-grammar? name)
  (not (bundled-language-grammar name)))

(define (plum/orphan-grammars installed)
  (filter plum/no-grammar? installed))

;;; A string argument wins; otherwise falls back to the current buffer's language.
(define (plum/resolve-grammar-arg pane cmd arg)
  (let ((name (call! "stdlib/resolve-lang-arg" pane cmd arg)))
    (cond ((not name) #f)
          ((not (bundled-language-grammar name))
           (log! 'info (string-append cmd ": no grammar for language \"" name "\"; Tab after :plum-install-grammar lists the languages that have one"))
           #f)
          (else name))))

;; ── Install pipeline ──────────────────────────────────────────────────────────

;;; See README.md's "Grammar install pipeline".
(define (plum/install-grammar name)
  (let* ((source  (or (bundled-language-grammar name)
                      (error (string-append "plum: no grammar for language \"" name "\""))))
         (subpath (hash-ref source 'subpath))
         (out-path (grammar-output-path name)))
    (plum/install-grammar-deps! name)
    (plum/install-repo! (hash-ref source 'url) (grammar-source-dir name) (hash-ref source 'rev)
      #:build
      (lambda (dir)
        (plum/fetch-query! name "highlights.scm" (path-join dir "highlights.scm"))
        (print! (string-append "Compiling grammar for " name "..."))
        (compile-grammar! (if (equal? subpath "") dir (path-join dir subpath)) out-path)
        (for-each
          (lambda (kind) (plum/try-fetch-query! name (cdr kind) dir))
          (grammar-optional-queries))))
    (register-grammar-from-sources! name out-path (hash-ref source 'symbol))))

;; ── Commands ──────────────────────────────────────────────────────────────────

;;; `:plum-install-grammar`'s own completion universe — see README.md's "Commands".
(register-completion-source! "plum:grammars"
  (lambda (id input cursor)
    (completion-emit! id (bundled-grammar-languages)))
  #:target 'minibuf #:match 'string)

(define-typed-command! "plum-install-grammar"
  "Install (or repair) a language's tree-sitter grammar, always from a clean re-clone (default: the current buffer's language)."
  (lambda (pane arg)
    (let ((name (plum/resolve-grammar-arg pane "plum-install-grammar" arg)))
      (when name
        (log! 'info (string-append "PLUM: installing grammar for " name))
        (with-handler
          (lambda (err)
            (log! 'error (string-append "PLUM: install failed: " (to-string err))))
          (plum/install-grammar name)))))
  #:inline-output #t #:complete "plum:grammars")

(define-command! "plum/ensure-grammars"
  "Install the grammars of the named languages (a list) that are not yet compiled."
  (lambda (grammars)
    (unless (and (list? grammars) (not (null? grammars)))
      (error "plum/ensure-grammars: requires a non-empty list of language names, e.g. (call! \"plum/ensure-grammars\" '(\"rust\" \"json\"))"))
    (let ((unknown (filter plum/no-grammar? grammars)))
      (unless (null? unknown)
        (error (string-append "plum/ensure-grammars: no grammar for "
                              (string-join unknown ", ")))))
    (let ((missing (filter plum/not-installed? grammars)))
      (if (null? missing)
          (log! 'info "PLUM: all requested grammars are installed")
          (plum/batch-run! "installed grammar" missing plum/install-grammar #:progress print!))))
  #:inline-output #t)

(define-typed-command! "plum-list-grammars"
  "Show the installed grammars and the orphan ones."
  (lambda (pane)
    (let* ((installed (installed-grammars))
           (orphans (plum/orphan-grammars installed)))
      (call! "stdlib/show-report!" pane "plum-grammars"
        (list (call! "stdlib/report-section" "Installed grammars"
                (filter (lambda (name) (not (member name orphans))) installed)
                #f)
              (call! "stdlib/report-section" "Orphan grammars" orphans
                "remove with :plum-cleanup-grammars"))))))

(define-typed-command! "plum-cleanup-grammars"
  "Delete compiled grammar files whose name is not a language with a bundled grammar."
  (lambda ()
    (let ((orphans (plum/orphan-grammars (installed-grammars))))
      (if (null? orphans)
          (log! 'info "PLUM: no orphan grammars to remove")
          (plum/batch-run! "removed grammar" orphans
            (lambda (name)
              (call! "stdlib/delete-file!" (grammar-output-path name))))))))
