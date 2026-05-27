;;; core:plum/grammars.scm — tree-sitter grammar installation pipeline.
;;;
;;; Provided procedures (callable from user init.scm via (call! …)):
;;;   plum/declare-grammar-source!  — declare a grammar name + source info
;;;   plum/ensure-grammars!         — register installed, queue missing for
;;;                                   startup auto-install if *plum-auto-install-grammars*
;;;
;;; Commands defined here:
;;;   :plum-install-grammar  — install grammar for current (or named) language
;;;   :plum-update-grammar   — re-clone and recompile (purges old source)
;;;   :plum-ensure-grammars  — install all missing declared grammars
;;;   :plum-list-grammars    — log installed / declared / orphan / missing
;;;   :plum-cleanup-grammars — delete orphan compiled grammars

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

;;; URL for the Helix-pinned highlights query for `name`.
(define (plum/helix-query-url name)
  (define helix-pin
    (call-with-input-file
      (path-join (runtime-dir) "scheme" "helix-pin.scm")
      read))
  (string-append
    "https://raw.githubusercontent.com/helix-editor/helix/"
    helix-pin
    "/runtime/queries/" name "/highlights.scm"))

;; ── Grammar discovery ─────────────────────────────────────────────────────────

;;; #t if the compiled grammar for `name` exists on disk.
(define (plum/grammar-installed? name)
  (path-exists? (grammar-output-path name)))

;;; Names of all compiled grammars on disk (filenames in <data>/grammars/
;;; matching .<ext> suffix, excluding the sources/ subdirectory).
(define (plum/installed-grammars)
  (let ((gdir (plum/grammars-dir)))
    (if (not (path-exists? gdir))
        '()
        (filter
          (lambda (name) (not (equal? name "sources")))
          (map (lambda (filename)
                 ;; Strip the platform extension to get the grammar name.
                 (let* ((dot (string-length filename))
                        (last-dot
                          (let search ((i (- dot 1)))
                            (cond ((< i 0) dot)
                                  ((equal? (substring filename i (+ i 1)) ".") i)
                                  (else (search (- i 1)))))))
                   (substring filename 0 last-dot)))
               (filter
                 (lambda (e) (and (plum/valid-dir-entry? e)
                                  (not (equal? e "sources"))))
                 (list-dir gdir)))))))

;;; Declared grammar names not yet compiled.
(define (plum/missing-grammars)
  (filter (lambda (name) (not (plum/grammar-installed? name)))
          (hash-keys->list *plum-grammar-sources*)))

;;; Compiled grammar files whose names are not in the declared source registry.
(define (plum/orphan-grammars)
  (filter (lambda (name) (not (hash-contains? *plum-grammar-sources* name)))
          (plum/installed-grammars)))

;; ── Install pipeline ──────────────────────────────────────────────────────────

;;; Install a single grammar from its declared source:
;;;   1. git-clone-rev  — blobless clone at pinned rev
;;;   2. curl-fetch     — download Helix highlights query
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
    (git-clone-rev url src-dir rev)
    (curl-fetch (plum/helix-query-url name) hl-path)
    (compile-grammar! build-dir out-path)
    (register-grammar! name out-path symbol hl-path)))

;; ── Startup auto-install control ─────────────────────────────────────────────

;;; Set to #f in init.scm to disable startup auto-install of missing grammars.
(define *plum-auto-install-grammars* #t)

;;; Called at plugin load time.  For each declared grammar:
;;;   - Already compiled → register-grammar! (queues PendingLanguageReg, no subprocess).
;;;   - Missing + auto-install enabled → queue-grammar-install! (signals the editor
;;;     to run the startup bracket after init.scm completes).
(define (plum/ensure-grammars!)
  (for-each
    (lambda (name)
      (let ((out  (grammar-output-path name))
            (hl   (plum/grammar-highlights-path name))
            (sym  (plum/grammar-source-symbol name)))
        (cond
          ((plum/grammar-installed? name)
           (when (path-exists? hl)
             (register-grammar! name out sym hl)))
          (*plum-auto-install-grammars*
           (queue-grammar-install! name)))))
    (hash-keys->list *plum-grammar-sources*)))

;; ── Commands ──────────────────────────────────────────────────────────────────

(define-command-inline-output! "plum-install-grammar"
  "Install the tree-sitter grammar for the current buffer's language (or NAME)."
  (lambda ()
    (let ((name (buffer-language (current-buffer))))
      (if (equal? name "")
          (log! 'warn "plum-install-grammar: current buffer has no language set")
          (begin
            (log! 'info (string-append "PLUM: installing grammar for " name))
            (with-handler
              (lambda (err)
                (log! 'error (string-append "PLUM: install failed: " (to-string err))))
              (plum/install-grammar name)))))))

(define-command-inline-output! "plum-update-grammar"
  "Re-clone and recompile the grammar for the current buffer's language."
  (lambda ()
    (let ((name (buffer-language (current-buffer))))
      (if (equal? name "")
          (log! 'warn "plum-update-grammar: current buffer has no language set")
          (begin
            (log! 'info (string-append "PLUM: updating grammar for " name))
            ;; Remove old source so git-clone-rev gets a clean slate.
            (let ((src-dir (plum/grammar-source-dir name)))
              (when (path-exists? src-dir)
                (delete-dir src-dir)))
            (with-handler
              (lambda (err)
                (log! 'error (string-append "PLUM: update failed: " (to-string err))))
              (plum/install-grammar name)))))))

(define-command-inline-output! "plum-ensure-grammars"
  "Install all declared grammars that are not yet compiled."
  (lambda ()
    (let ((missing (plum/missing-grammars)))
      (if (null? missing)
          (log! 'info "PLUM: all declared grammars are installed")
          (plum/batch-run "installed grammar" missing plum/install-grammar)))))

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
