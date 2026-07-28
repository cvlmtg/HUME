;;; core:plum/grammars.scm — grammar INSTALL pipeline only. The source
;;; catalog, path helpers, and startup registration of already-compiled
;;; grammars live in core (runtime/scheme/grammars.scm) — this file only
;;; runs when the user explicitly asks PLUM to install/manage a grammar.

(require "lib.scm")

;;; Helix commit pin, read once at plugin load.
(define *plum-helix-pin*
  (call-with-input-file (path-join (runtime-dir) "scheme" "helix-pin.scm") read))

;;; URL for the Helix-pinned query file `filename` (e.g. "highlights.scm",
;;; "injections.scm") for `name`.
(define (plum/helix-query-url name filename)
  (string-append "https://raw.githubusercontent.com/helix-editor/helix/"
                 *plum-helix-pin* "/runtime/queries/" name "/" filename))

;; ── Query inheritance resolution ──────────────────────────────────────────────
;; A query file can declare `; inherits: dep,dep,...` instead of writing its
;; own patterns; tree-sitter has no notion of this, so `plum/resolve-query`
;; recursively fetches and prepends each named dependency's copy before
;; anything reaches tree-sitter. No deduplication — see README.

;;; #t if `line`, trimmed, is an `; inherits: a,b,c` directive.
(define (plum/inherits-line? line)
  (starts-with? (trim line) "; inherits:"))

;;; Dependency names out of a `; inherits: a,b,c` line.
(define (plum/inherits-deps line)
  (let ((trimmed (trim line)))
    (map trim (split-many (trim (substring trimmed 11 (string-length trimmed))) ","))))

;;; #t if `name` is safe to use as one filesystem path segment: no `.`/`..`
;;; and no path separator. Guards `plum/fetch-raw-query`'s scratch-file path
;;; against a dependency name parsed from a downloaded query file's
;;; `; inherits:` line — untrusted content, unlike the top-level grammar
;;; name (which always comes from the fixed catalog).
(define (plum/safe-segment? name)
  (and (not (equal? name "."))
       (not (equal? name ".."))
       (not (string-contains? name "/"))
       (not (string-contains? name "\\"))))

;;; Fetch `name`'s `filename` query to a scratch file and return its raw
;;; content. `curl` is deliberately NOT wrapped in a `with-handler`: this
;;; runs inside `plum/resolve-query`'s tolerant handler, and re-raising a
;;; native-builtin error from an inner handler into an outer one corrupts
;;; Steel 0.8.2's continuation stack (pinned by `steel_stdlib_availability`).
;;; Cost: a failed curl may leave a stale `tmp` — overwritten next attempt.
(define (plum/fetch-raw-query name filename)
  (unless (plum/safe-segment? name)
    ;; `plum/resolve-query`'s tolerant handler (used for every dependency
    ;; below the top level) swallows this raise the same as an ordinary
    ;; 404 — log it here so a real path-traversal attempt still leaves a
    ;; trace instead of silently resolving to "no query for this dependency".
    (log! 'warn (string-append "plum/fetch-raw-query: rejecting unsafe grammar/dependency name \"" name "\""))
    (error (string-append "plum/fetch-raw-query: unsafe grammar/dependency name \"" name "\"")))
  (let ((tmp (path-join (grammar-sources-dir) (string-append "_fetch_" name "_" filename))))
    (run-inline-output! "curl" (list "-fsSL" "-o" tmp "--" (plum/helix-query-url name filename)))
    (let ((content (with-handler
                     (lambda (err) (plum/delete-file tmp) (raise-error err))
                     (plum/read-file tmp))))
      (plum/delete-file tmp)
      content)))

;;; Fetch `name`'s `filename` query and fully resolve any `; inherits:` chain
;;; into a single string with no dangling directives. When `tolerant?` is
;;; true, a missing file at this level resolves to `""` instead of raising —
;;; a dependency need not ship every query kind its inheritor asks for (e.g.
;;; a dependency named only in `tsx`'s injections.scm inherits line may have
;;; no injections.scm of its own). The top-level call is never tolerant — a
;;; genuinely missing query for the grammar itself should still raise, so
;;; callers like `plum/try-fetch-injections!` can tell the difference.
(define (plum/resolve-query name filename tolerant?)
  (let ((content (if tolerant?
                      (with-handler (lambda (err) #f) (plum/fetch-raw-query name filename))
                      (plum/fetch-raw-query name filename))))
    (if (not content)
        ""
        (let* ((lines (split-many content "\n"))
               (inherits-line (plum/find plum/inherits-line? lines)))
          (if inherits-line
              (string-join
                (append
                  (map (lambda (dep) (plum/resolve-query dep filename #t))
                       (plum/inherits-deps inherits-line))
                  (list (string-join (filter (lambda (l) (not (equal? l inherits-line))) lines) "\n")))
                "\n")
              content)))))

;;; Fetch and fully resolve `name`'s `filename` query, writing the result to
;;; `dest`. Raises on a 404 for the grammar's own query.
(define (plum/fetch-query! name filename dest)
  (plum/write-file dest (plum/resolve-query name filename #f)))

;; ── Injection dependencies ────────────────────────────────────────────────────

;;; Hash: name → (dep-name ...). See README.md § grammar dependencies.
(define *plum-grammar-deps*
  (hash "markdown" (list "markdown.inline")))

(define (plum/grammar-deps name)
  (if (hash-contains? *plum-grammar-deps* name)
      (hash-ref *plum-grammar-deps* name)
      '()))

;;; Install any not-yet-compiled dependency grammars for `name` first.
(define (plum/install-grammar-deps! name)
  (for-each
    (lambda (dep)
      (unless (grammar-installed? dep)
        (log! 'info (string-append "PLUM: installing dependency " dep " for " name))
        (plum/install-grammar dep)))
    (plum/grammar-deps name)))

;;; Fetch `name`'s injections.scm to its declared path, tolerating a missing
;;; file (most grammars have none) — a 404 makes `plum/fetch-query!` raise,
;;; which would otherwise abort the whole grammar install for no reason.
;;; Returns the path on success, `#f` if there is no injections query to fetch.
(define (plum/try-fetch-injections! name)
  (let ((path (grammar-injections-path name)))
    (with-handler
      (lambda (err)
        (log! 'trace (string-append "PLUM: no injections.scm for " name " (" (to-string err) ")"))
        #f)
      (begin
        (plum/fetch-query! name "injections.scm" path)
        path))))

;; ── Grammar discovery ─────────────────────────────────────────────────────────

;;; #t if `name` has no compiled grammar on disk yet.
(define (plum/not-installed? name)
  (not (grammar-installed? name)))

;;; Declared grammar names not yet compiled.
(define (plum/missing-grammars)
  (filter plum/not-installed? (grammar-source-names)))

;;; Compiled grammar files whose names are not in the declared source registry.
(define (plum/orphan-grammars)
  (filter (lambda (name) (not (grammar-source-known? name)))
          (installed-grammars)))

;;; Resolve the target grammar for a `:` grammar command: a string argument
;;; wins; otherwise fall back to the current buffer's language. Returns the
;;; name, or #f after logging a warning. `arg` is a string only when the user
;;; typed one — the minibuffer passes the default count 1 otherwise.
(define (plum/resolve-grammar-arg cmd arg)
  (let ((name (if (string? arg) arg (buffer-language (current-buffer)))))
    (cond ((not (string? name))
           (log! 'warn (string-append cmd ": no grammar name given and current buffer has no language set"))
           #f)
          ((not (grammar-source-known? name))
           (log! 'warn (string-append cmd ": unknown grammar \"" name "\" — see :plum-list-grammars"))
           #f)
          (else name))))

;; ── Install pipeline ──────────────────────────────────────────────────────────

;;; Install a single grammar from its declared source, always from a clean
;;; slate — this doubles as the repair path for a grammar left in a failed
;;; state. See README "Grammar install pipeline" for the numbered steps.
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
    (plum/delete-dir src-dir)
    ;; Blobless clone (skip file-history blobs) at the pinned rev, then
    ;; checkout that exact revision.
    (run-inline-output! "git" (list "clone" "--filter=blob:none" "--" url src-dir))
    (run-inline-output! "git" (list "checkout" "--force" "--end-of-options" rev "--") #:cwd src-dir)
    (plum/fetch-query! name "highlights.scm" hl-path)
    ;; git prints its own progress; the C compiler stays silent until it's
    ;; done or errors, which on a slow grammar reads as a hang.
    (displayln (string-append "Compiling grammar for " name "..."))
    (compile-grammar! build-dir out-path)
    (register-grammar! name out-path symbol hl-path (plum/try-fetch-injections! name))))

;; ── Commands ──────────────────────────────────────────────────────────────────

(define-command! "plum-install-grammar"
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

(define-command! "plum-list-grammars"
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

(define-command! "plum-cleanup-grammars"
  "Delete compiled grammar files that are no longer in the declared source list."
  (lambda ()
    (let ((orphans (plum/orphan-grammars)))
      (if (null? orphans)
          (log! 'info "PLUM: no orphan grammars to remove")
          (plum/batch-run "removed grammar" orphans
            (lambda (name)
              (plum/delete-file (grammar-output-path name))))))))
