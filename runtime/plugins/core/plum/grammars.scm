;;; core:plum/grammars.scm — tree-sitter grammar installation pipeline.
;;;
;;; Provided procedures:
;;;   plum/declare-grammar-source!         — declare a grammar name + source info
;;;   plum/register-installed-grammars!    — register already-compiled grammars (passive)
;;;
;;; Commands defined here:
;;;   :plum-install-grammar  — install a named grammar, or the current buffer's
;;;   :plum-update-grammar   — re-clone and recompile (purges old source)
;;;   :plum-ensure-grammars  — install named grammars not yet compiled (list required)
;;;   :plum-list-grammars    — log installed / declared / orphan / missing
;;;   :plum-cleanup-grammars — delete orphan compiled grammars

(require "lib.scm")
(provide plum/declare-grammar-source! plum/register-installed-grammars!)

;; ── Grammar source registry ───────────────────────────────────────────────────

;;; Hash: name → (url rev symbol subpath)
(define *plum-grammar-sources* (hash))

;;; Register a grammar source from a 5-tuple (name url rev symbol subpath).
(define (plum/declare-grammar-source! entry)
  (let ((name    (list-ref entry 0))
        (url     (list-ref entry 1))
        (rev     (list-ref entry 2))
        (symbol  (list-ref entry 3))
        (subpath (list-ref entry 4)))
    (set! *plum-grammar-sources*
          (hash-insert *plum-grammar-sources* name (list url rev symbol subpath)))))

;;; Accessors.
(define (plum/grammar-source-url name)
  (list-ref (hash-ref *plum-grammar-sources* name) 0))
(define (plum/grammar-source-rev name)
  (list-ref (hash-ref *plum-grammar-sources* name) 1))
(define (plum/grammar-source-symbol name)
  (list-ref (hash-ref *plum-grammar-sources* name) 2))
(define (plum/grammar-source-subpath name)
  (list-ref (hash-ref *plum-grammar-sources* name) 3))

;; ── Path helpers ──────────────────────────────────────────────────────────────

(define (plum/grammars-dir)
  (path-join (data-dir) "grammars"))

(define (plum/grammar-sources-dir)
  (path-join (plum/grammars-dir) "sources"))

(define (plum/grammar-source-dir name)
  (path-join (plum/grammar-sources-dir) name))

(define (plum/grammar-highlights-path name)
  (path-join (plum/grammar-source-dir name) "highlights.scm"))

(define (plum/grammar-injections-path name)
  (path-join (plum/grammar-source-dir name) "injections.scm"))

;;; Helix commit pin, read once at plugin load.
(define *plum-helix-pin*
  (call-with-input-file (path-join (runtime-dir) "scheme" "helix-pin.scm") read))

;;; URL for the Helix-pinned query file `filename` (e.g. "highlights.scm",
;;; "injections.scm") for `name`.
(define (plum/helix-query-url name filename)
  (string-append "https://raw.githubusercontent.com/helix-editor/helix/"
                 *plum-helix-pin* "/runtime/queries/" name "/" filename))

;; ── Injection dependencies ────────────────────────────────────────────────────

;;; Hash: name → (dep-name ...). Grammars whose injections only resolve if
;;; another grammar is also compiled and attached — e.g. markdown's
;;; `(inline)` injection resolves to the language "markdown.inline" (the
;;; same name as its languages.scm identity and its grammar-sources.scm /
;;; Helix query-directory key — no renaming needed), so without that
;;; grammar compiled and attached, bold/italic/inline-code never highlight
;;; even though markdown itself installed cleanly.
(define *plum-grammar-deps*
  (hash "markdown" (list "markdown.inline")))

(define (plum/grammar-deps name)
  (if (hash-contains? *plum-grammar-deps* name)
      (hash-ref *plum-grammar-deps* name)
      '()))

;;; Install any not-yet-compiled dependency grammars for `name` before `name`
;;; itself installs. Runs before the main install steps so a fresh
;;; `:plum-install-grammar` on a markdown buffer transparently pulls in
;;; markdown.inline too — the user never needs to know it exists.
(define (plum/install-grammar-deps! name)
  (for-each
    (lambda (dep)
      (unless (plum/grammar-installed? dep)
        (log! 'info (string-append "PLUM: installing dependency " dep " for " name))
        (plum/install-grammar dep)))
    (plum/grammar-deps name)))

;;; Fetch `name`'s injections.scm to its declared path, tolerating a missing
;;; file (most grammars have none) — a 404 makes `curl-fetch` raise, which
;;; would otherwise abort the whole grammar install for no reason. Returns
;;; the path on success, `#f` if there is no injections query to fetch.
(define (plum/try-fetch-injections! name)
  (let ((path (plum/grammar-injections-path name)))
    (with-handler
      (lambda (err)
        (log! 'trace (string-append "PLUM: no injections.scm for " name " (" (to-string err) ")"))
        #f)
      (begin
        (curl-fetch (plum/helix-query-url name "injections.scm") path)
        path))))

;; ── Grammar discovery ─────────────────────────────────────────────────────────

;;; #t if the compiled grammar for `name` exists on disk.
(define (plum/grammar-installed? name)
  (path-exists? (grammar-output-path name)))

;;; Strip the platform extension from `filename`, returning the grammar name,
;;; or `#f` if the file has no extension (e.g. "sources" dir, dotfiles like
;;; ".DS_Store" where the dot is at position 0).
(define (plum/grammar-name-from-file filename)
  (let* ((len (string-length filename))
         (last-dot
           (let search ((i (- len 1)))
             (cond ((<= i 0) -1)
                   ((equal? (substring filename i (+ i 1)) ".") i)
                   (else (search (- i 1)))))))
    (if (> last-dot 0) (substring filename 0 last-dot) #f)))

;;; Names of all compiled grammars on disk (filenames in <data>/grammars/
;;; with a real extension, excluding the sources/ subdirectory and dotfiles).
(define (plum/installed-grammars)
  (let ((gdir (plum/grammars-dir)))
    (if (not (path-exists? gdir))
        '()
        (filter (lambda (x) x)
                (map plum/grammar-name-from-file
                     (filter plum/valid-dir-entry? (list-dir gdir)))))))

;;; Declared grammar names not yet compiled.
(define (plum/missing-grammars)
  (filter (lambda (name) (not (plum/grammar-installed? name)))
          (hash-keys->list *plum-grammar-sources*)))

;;; Compiled grammar files whose names are not in the declared source registry.
(define (plum/orphan-grammars)
  (filter (lambda (name) (not (hash-contains? *plum-grammar-sources* name)))
          (plum/installed-grammars)))

;;; Resolve the target grammar for a `:` grammar command: a string argument
;;; wins; otherwise fall back to the current buffer's language. Returns the
;;; name, or #f after logging a warning. `arg` is a string only when the user
;;; typed one — the minibuffer passes the default count 1 otherwise.
(define (plum/resolve-grammar-arg cmd arg)
  (let ((name (if (string? arg) arg (buffer-language (current-buffer)))))
    (cond ((not (string? name))
           (log! 'warn (string-append cmd ": no grammar name given and current buffer has no language set"))
           #f)
          ((not (hash-contains? *plum-grammar-sources* name))
           (log! 'warn (string-append cmd ": unknown grammar \"" name "\" — see :plum-list-grammars"))
           #f)
          (else name))))

;; ── Install pipeline ──────────────────────────────────────────────────────────

;;; Install a single grammar from its declared source:
;;;   0. plum/install-grammar-deps! — install any dependency grammars first
;;;   1. git-clone-rev  — blobless clone at pinned rev
;;;   2. curl-fetch     — download Helix highlights query
;;;   2b. plum/try-fetch-injections! — download Helix injections query, if any
;;;   3. compile-grammar! — tree-sitter build → shared lib
;;;   4. register-grammar! — attach to language in this session
(define (plum/install-grammar name)
  (let* ((url     (plum/grammar-source-url name))
         (rev     (plum/grammar-source-rev name))
         (symbol  (plum/grammar-source-symbol name))
         (subpath (plum/grammar-source-subpath name))
         (src-dir (plum/grammar-source-dir name))
         (build-dir (if (equal? subpath "")
                        src-dir
                        (path-join src-dir subpath)))
         (out-path (grammar-output-path name))
         (hl-path  (plum/grammar-highlights-path name)))
    (plum/install-grammar-deps! name)
    (git-clone-rev url src-dir rev)
    (curl-fetch (plum/helix-query-url name "highlights.scm") hl-path)
    (compile-grammar! build-dir out-path)
    (register-grammar! name out-path symbol hl-path (plum/try-fetch-injections! name))))

;; ── Startup grammar registration ─────────────────────────────────────────────

;;; Called at plugin load time.  For each declared grammar that is already
;;; compiled on disk, call register-grammar! (no subprocess).  Missing grammars
;;; are silently skipped — the user opts in to auto-install via:
;;;   (call! "plum-ensure-grammars" '("rust" "json"))  ; in init.scm, list required
(define (plum/register-installed-grammars!)
  (for-each
    (lambda (name)
      (let ((out  (grammar-output-path name))
            (hl   (plum/grammar-highlights-path name))
            (inj  (plum/grammar-injections-path name))
            (sym  (plum/grammar-source-symbol name)))
        (when (and (plum/grammar-installed? name) (path-exists? hl))
          (register-grammar! name out sym hl (if (path-exists? inj) inj #f)))))
    (hash-keys->list *plum-grammar-sources*)))

;; ── Commands ──────────────────────────────────────────────────────────────────

(define-command! "plum-install-grammar"
  "Install a tree-sitter grammar by name (default: the current buffer's language)."
  (lambda (arg)
    (let ((name (plum/resolve-grammar-arg "plum-install-grammar" arg)))
      (when name
        (log! 'info (string-append "PLUM: installing grammar for " name))
        (with-handler
          (lambda (err)
            (log! 'error (string-append "PLUM: install failed: " (to-string err))))
          (plum/install-grammar name)))))
  #:inline-output #t)

(define-command! "plum-update-grammar"
  "Re-clone and recompile a tree-sitter grammar by name (default: the current buffer's language)."
  (lambda (arg)
    (let ((name (plum/resolve-grammar-arg "plum-update-grammar" arg)))
      (when name
        (log! 'info (string-append "PLUM: updating grammar for " name))
        ;; Remove old source so git-clone-rev gets a clean slate.
        (let ((src-dir (plum/grammar-source-dir name)))
          (when (path-exists? src-dir)
            (delete-dir src-dir)))
        (with-handler
          (lambda (err)
            (log! 'error (string-append "PLUM: update failed: " (to-string err))))
          (plum/install-grammar name)))))
  #:inline-output #t)

(define-command! "plum-ensure-grammars"
  "Install the named grammars (a list) that are not yet compiled."
  (lambda (grammars)
    (unless (and (list? grammars) (not (null? grammars)))
      (error "plum-ensure-grammars: requires a non-empty list of grammar names, e.g. (call! \"plum-ensure-grammars\" '(\"rust\" \"json\"))"))
    (let ((missing (filter (lambda (name) (not (plum/grammar-installed? name))) grammars)))
      (if (null? missing)
          (log! 'info "PLUM: all requested grammars are installed")
          (plum/batch-run "installed grammar" missing plum/install-grammar))))
  #:inline-output #t)

(define-command! "plum-list-grammars"
  "Log declared, installed, orphan, and missing grammar lists."
  (lambda ()
    (let ((declared  (hash-keys->list *plum-grammar-sources*))
          (installed (plum/installed-grammars))
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
              (delete-file (grammar-output-path name))))))))
