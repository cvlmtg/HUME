;;; core:plum/grammars.scm

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

;; ── Query inheritance resolution ──────────────────────────────────────────────
;;
;; A query file can declare `; inherits: dep,dep,...` instead of writing out
;; its own patterns — a directive naming other query sources whose patterns
;; should be spliced in. tree-sitter itself has no notion of this; a query
;; source containing only that line compiles as a valid but empty query.
;; `plum/resolve-query` resolves the chain before anything reaches
;; tree-sitter: it fetches `name`'s copy of the file, and whenever it finds
;; an `; inherits:` line, recursively fetches and prepends each named
;; dependency's copy of the same file.

;;; #t if `line`, trimmed, is an `; inherits: a,b,c` directive.
(define (plum/inherits-line? line)
  (starts-with? (trim line) "; inherits:"))

;;; Dependency names out of a `; inherits: a,b,c` line.
(define (plum/inherits-deps line)
  (let ((trimmed (trim line)))
    (map trim (split-many (trim (substring trimmed 11 (string-length trimmed))) ","))))

;;; First element of `lst` satisfying `pred?`, or `#f`.
(define (plum/find pred? lst)
  (cond ((null? lst) #f)
        ((pred? (car lst)) (car lst))
        (else (plum/find pred? (cdr lst)))))

;;; Fetch `name`'s `filename` query to a scratch file and return its raw
;;; content as a string.
(define (plum/fetch-raw-query name filename)
  (let ((tmp (path-join (plum/grammar-sources-dir) (string-append "_fetch_" name "_" filename))))
    (curl-fetch (plum/helix-query-url name filename) tmp)
    (let ((content (read-file tmp)))
      (delete-file tmp)
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
;;; `dest`. Drop-in replacement for a plain `curl-fetch` of a query file —
;;; same 404 behaviour (raises), but also resolves `; inherits:` chains.
(define (plum/fetch-query! name filename dest)
  (write-file dest (plum/resolve-query name filename #f)))

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
        (plum/fetch-query! name "injections.scm" path)
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

;;; Install a single grammar from its declared source, always from a clean
;;; slate — this doubles as the repair path for a grammar left in a failed
;;; state (e.g. a source tree cloned but never compiled):
;;;   0. plum/install-grammar-deps! — install any dependency grammars first
;;;   1. delete-dir     — purge any existing source tree
;;;   2. git-clone-rev  — blobless clone at pinned rev
;;;   3. plum/fetch-query! — download highlights query, resolving any
;;;      `; inherits:` chain (see "Query inheritance resolution" above)
;;;   4. plum/try-fetch-injections! — download Helix injections query, if any
;;;   5. compile-grammar! — tree-sitter build → shared lib (preceded by a
;;;      displayln status line — the C compiler itself is silent)
;;;   6. register-grammar! — attach to language in this session
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
    ;; git-clone-rev refuses a non-empty dest — clear any stale source tree
    ;; (e.g. left behind by a prior install that failed after cloning) first.
    ;; delete-dir is a no-op when src-dir doesn't exist.
    (delete-dir src-dir)
    (git-clone-rev url src-dir rev)
    (plum/fetch-query! name "highlights.scm" hl-path)
    ;; git prints its own progress; the C compiler stays silent until it's
    ;; done or errors, which on a slow grammar reads as a hang.
    (displayln (string-append "Compiling grammar for " name "..."))
    (compile-grammar! build-dir out-path)
    (register-grammar! name out-path symbol hl-path (plum/try-fetch-injections! name))))

;; ── Startup grammar registration ─────────────────────────────────────────────

;;; Passive: registers already-compiled grammars only, no subprocess. See README.md.
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
