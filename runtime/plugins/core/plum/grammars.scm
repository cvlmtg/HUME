;;; core:plum/grammars.scm — the grammar install pipeline; see README.md.

(require "lib/helpers.scm")
(require-builtin steel/time)

;; ── Grammar source ────────────────────────────────────────────────────────────
;; See README.md's "Grammar source".

(define *plum-grammar-tag*
  (read-data-file (path-join (plugin-dir) "data" "treesitter-hume-pin.scm")))

;;; The folder of grammar archives `#:config 'grammar-source` names, or #f for none.
(define *plum-grammar-folder*
  (let ((cfg (plugin-config)))
    (and (hash-contains? cfg 'grammar-source)
         (let ((folder (call! "stdlib/config-string" "core:plum" cfg 'grammar-source "")))
           (unless (call! "stdlib/path-absolute?" folder)
             (error (string-append "plum: 'grammar-source must be an absolute path: " folder)))
           folder))))

(define (plum/release-url file)
  (string-append "https://github.com/cvlmtg/treesitter.hume/releases/download/"
                 *plum-grammar-tag* "/" file))

(define (plum/bundle-file ext)
  (string-append "grammars-" (grammar-platform) "." ext))

(define (plum/grammar-staging-dir)
  (path-join (plum/staging-dir) "grammars"))

(define (plum/download-text url)
  (let ((tmp (path-join (plum/grammar-staging-dir) "index")))
    (create-directory! (plum/grammar-staging-dir))
    (with-handler
      (lambda (err)
        (call! "stdlib/delete-file!" tmp)
        (raise-error err))
      (call! "stdlib/download!" url tmp)
      (let ((text (call-with-input-file tmp read-port-to-string)))
        (call! "stdlib/delete-file!" tmp)
        text))))

;; ── Index ─────────────────────────────────────────────────────────────────────
;; See README.md's "Grammar source".

(define (plum/index-error reason)
  (error (string-append "plum: the grammar index " reason)))

(define (plum/parse-member line)
  (let ((fields (split-whitespace line)))
    (unless (= (length fields) 4)
      (plum/index-error (string-append "has a malformed line: " line)))
    (let ((offset (string->number (list-ref fields 1)))
          (size (string->number (list-ref fields 2))))
      (unless (and (integer? offset) (integer? size) (>= offset 0) (> size 0))
        (plum/index-error (string-append "has a malformed line: " line)))
      (list (car fields) offset size (list-ref fields 3)))))

;;; The release index's `(name offset size sha256)` lines.
(define (plum/parse-index text)
  (let* ((lines (filter (lambda (line) (not (equal? (trim line) "")))
                        (split-many text "\n")))
         (header (if (null? lines) '() (split-whitespace (car lines)))))
    (unless (and (= (length header) 3) (equal? (car header) "treesitter.hume"))
      (plum/index-error "does not start with a treesitter.hume header"))
    (unless (equal? (list-ref header 2) (grammar-platform))
      (plum/index-error (string-append "is for " (list-ref header 2) ", not "
                                       (grammar-platform))))
    (map plum/parse-member (cdr lines))))

(define (plum/read-release-index)
  (plum/parse-index
    (with-handler
      (lambda (err)
        (error (string-append "plum: cannot fetch the treesitter.hume " *plum-grammar-tag*
                              " grammar index for " (grammar-platform) ": "
                              (to-string err))))
      (plum/download-text (plum/release-url (plum/bundle-file "index"))))))

;;; Calls `use` on `name`'s release archive, downloaded and digest-checked, then deletes it.
(define (plum/with-release-member! name member use)
  (let ((dest (path-join (plum/grammar-staging-dir) (string-append name ".tar.gz"))))
    (with-handler
      (lambda (err)
        (call! "stdlib/delete-file!" dest)
        (raise-error err))
      (call! "stdlib/download!" (plum/release-url (plum/bundle-file "tar")) dest
             (list-ref member 1) (list-ref member 2))
      (call! "stdlib/verify-sha256!" dest (list-ref member 3))
      (use dest))
    (call! "stdlib/delete-file!" dest)))

;;; The configured folder, or #f when none is configured. See README.md's "Grammar source".
(define (plum/checked-folder)
  (let* ((folder *plum-grammar-folder*)
         (fail (lambda (reason)
                 (error (string-append "plum: 'grammar-source " folder " " reason)))))
    (when folder
      (unless (is-dir? folder)
        (fail "is not a directory"))
      (for-each
        (lambda (managed)
          (unless (first-path-outside managed (list folder))
            (fail (string-append "is inside " managed ", which PLUM manages"))))
        (list (plum/staging-dir) (grammars-dir)))
      (let ((archives (map (lambda (name) (string-append name ".tar.gz")) (bundled-languages))))
        (when (null? (filter (lambda (entry) (member entry archives)) (dir-entry-names folder)))
          (fail "holds no grammar archive"))))
    folder))

;;; See README.md's "Grammar source".
(define (plum/source-lookup)
  (let ((folder (plum/checked-folder))
        (release #f))
    (lambda (name)
      (let ((archive (and folder (path-join folder (string-append name ".tar.gz")))))
        (if (and archive (path-exists? archive))
            (lambda (use) (use archive))
            (begin
              (unless release
                (set! release
                  (with-handler
                    (lambda (err)
                      (if folder
                          (error (string-append "plum: no " name " grammar in " folder "; "
                                                (to-string err)))
                          (raise-error err)))
                    (plum/read-release-index))))
              (let ((member (assoc name release)))
                (and member
                     (lambda (use) (plum/with-release-member! name member use))))))))))

;; ── Install pipeline ──────────────────────────────────────────────────────────
;; See README.md's "Grammar install pipeline".

(define (plum/check-unpacked! name dir)
  (let* ((manifest (read-data-file (path-join dir "manifest.scm")))
         (field (lambda (key) (grammar-manifest-ref manifest key)))
         (symbol (field 'symbol))
         (fail (lambda (reason) (error (string-append "plum: the " name " grammar " reason)))))
    (unless (equal? (field 'name) name)
      (fail (string-append "archive is the " (to-string (field 'name)) " grammar")))
    (unless (and (string? symbol) (not (equal? symbol "")))
      (fail "manifest names no symbol"))
    (unless (equal? (field 'platform) (grammar-platform))
      (fail (string-append "is built for " (to-string (field 'platform)))))
    (for-each
      (lambda (file)
        (unless (path-exists? (path-join dir file))
          (fail (string-append "archive has no " file))))
      (list (grammar-library-file name) (grammar-query-file "highlights.scm")))
    (unless (equal? (field 'tag) *plum-grammar-tag*)
      (log! 'warning (string-append "PLUM: the " name " grammar is from treesitter.hume "
                                    (to-string (field 'tag)) "; HUME pins "
                                    *plum-grammar-tag*)))))

;;; Hash: name → (dep-name ...). See README.md's "Grammar dependencies".
(define *plum-grammar-deps*
  (hash "markdown" (list "markdown.inline")))

(define (plum/grammar-deps name)
  (if (hash-contains? *plum-grammar-deps* name)
      (hash-ref *plum-grammar-deps* name)
      '()))

;;; A name for the next install folder after `installs`, a `grammar-installs` list.
(define (plum/new-install-name installs)
  (let ((now (current-milliseconds)))
    (number->string
      (if (null? installs)
          now
          (max now (+ 1 (string->number (last installs))))))))

;;; Deletes `path` with `delete!`, reporting a failure; see README.md's "Grammar install pipeline".
(define (plum/delete-older! path delete!)
  (with-handler
    (lambda (err)
      (let ((failure (string-append "PLUM: could not delete " path ": " (to-string err))))
        (if (equal? (current-os!) "windows")
            (log! 'warning (string-append failure ". A grammar library HUME has loaded cannot be "
                                          "deleted on Windows: restart HUME and run "
                                          ":plum-cleanup-grammars"))
            (log! 'error failure))))
    (delete! path)))

;;; Installs `name` from the source `plan`, a `plum/served-plan` result, resolved for it.
(define (plum/install-one plan name)
  (let* ((stage (path-join (plum/grammar-staging-dir) name))
         (source (cdr (assoc name plan)))
         (older (grammar-installs name))
         (install (plum/new-install-name older))
         (folder (path-join (grammar-dir name) install))
         (copy (path-join (plum/grammar-staging-dir) (string-append name "-" install)))
         (delete-copy! (lambda ()
                         (plum/delete-older! copy (lambda (path)
                                                     (call! "stdlib/delete-dir!" path))))))
    (call! "stdlib/delete-dir!" stage)
    (create-directory! (plum/grammar-staging-dir))
    (with-handler
      (lambda (err)
        (call! "stdlib/delete-dir!" stage)
        (delete-copy!)
        (raise-error err))
      (source (lambda (archive) (call! "stdlib/extract-archive!" "tar" archive stage)))
      (plum/check-unpacked! name stage)
      (copy-directory-recursively! stage copy)
      (check-grammar! name copy (grammar-library-path copy name))
      (replace-dir! stage folder))
    (delete-copy!)
    (register-installed-grammar! name folder)
    (for-each (lambda (old) (plum/delete-older! (path-join (grammar-dir name) old) remove-dir!))
              older)))

;; ── Grammar discovery ─────────────────────────────────────────────────────────

(define (plum/not-installed? name)
  (not (grammar-installed? name)))

;;; `plan`, newest first, with `name` after the not-installed dependencies it needs.
(define (plum/plan-add plan name)
  (if (member name plan)
      plan
      (cons name (plum/plan-add-all plan (filter plum/not-installed? (plum/grammar-deps name))))))

(define (plum/plan-add-all plan names)
  (if (null? names)
      plan
      (plum/plan-add-all (plum/plan-add plan (car names)) (cdr names))))

;;; `((name . source) ...)` in install order; see README.md's "Grammar install pipeline".
(define (plum/served-plan lookup names)
  (let* ((plan (map (lambda (name) (cons name (lookup name)))
                    (reverse (plum/plan-add-all '() names))))
         (absent (map car (filter (lambda (entry) (not (cdr entry))) plan))))
    (unless (null? absent)
      (error (string-append "plum: no " (string-join absent ", ") " grammar in "
                            (if *plum-grammar-folder*
                                (string-append *plum-grammar-folder* " or ")
                                "")
                            "treesitter.hume " *plum-grammar-tag* " for " (grammar-platform))))
    plan))

;;; Paths relative to `<data>/grammars/`; see README.md's "Commands".
(define (plum/grammar-leftovers in-use)
  (call! "stdlib/concat"
         (map (lambda (entry)
                (let ((installed (assoc entry in-use)))
                  (if installed
                      (let ((current (file-name (cdr installed))))
                        (map (lambda (inner) (path-join entry inner))
                             (filter (lambda (inner) (not (equal? inner current)))
                                     (dir-entry-names (grammar-dir entry)))))
                      (list entry))))
              (dir-entry-names (grammars-dir)))))

;;; See README.md's "Commands".
(define (plum/staging-leftovers)
  (dir-entry-names (plum/grammar-staging-dir)))

;;; A string argument wins; otherwise falls back to the current buffer's language.
(define (plum/resolve-grammar-arg pane cmd arg)
  (let ((name (call! "stdlib/resolve-lang-arg" pane cmd arg)))
    (cond ((not name) #f)
          ((not (member name (bundled-languages)))
           (log! 'info (string-append cmd ": no language \"" name "\"; Tab after :plum-install-grammar lists the languages"))
           #f)
          (else name))))

;; ── Commands ──────────────────────────────────────────────────────────────────

;;; `:plum-install-grammar`'s own completion universe — see README.md's "Commands".
(register-completion-source! "plum:grammars"
  (lambda (id input cursor)
    (completion-emit! id (bundled-languages)))
  #:target 'minibuf #:match 'string)

(define-typed-command! "plum-install-grammar"
  "Install (or repair) a language's tree-sitter grammar from the pinned treesitter.hume release (default: the current buffer's language)."
  (lambda (pane arg)
    (let ((name (plum/resolve-grammar-arg pane "plum-install-grammar" arg)))
      (when name
        (log! 'info (string-append "PLUM: installing grammar for " name))
        (with-handler
          (lambda (err)
            (log! 'error (string-append "PLUM: install failed: " (to-string err))))
          (let ((plan (plum/served-plan (plum/source-lookup) (list name))))
            (for-each
              (lambda (each)
                (unless (equal? each name)
                  (log! 'info (string-append "PLUM: installing dependency " each " for " name)))
                (plum/install-one plan each))
              (map car plan)))))))
  #:inline-output #t #:complete "plum:grammars")

(define-command! "plum/ensure-grammars"
  "Install the grammars of the named languages (a list) that are not yet installed."
  (lambda (grammars)
    (unless (and (list? grammars) (not (null? grammars)))
      (error "plum/ensure-grammars: requires a non-empty list of language names, e.g. (call! \"plum/ensure-grammars\" '(\"rust\" \"json\"))"))
    (let* ((languages (bundled-languages))
           (unknown (filter (lambda (name) (not (member name languages))) grammars)))
      (unless (null? unknown)
        (error (string-append "plum/ensure-grammars: no language "
                              (string-join unknown ", ")))))
    (let ((missing (filter plum/not-installed? grammars)))
      (if (null? missing)
          (log! 'info "PLUM: all requested grammars are installed")
          (let ((plan (plum/served-plan (plum/source-lookup) missing)))
            (plum/batch-run! "installed grammar" (map car plan)
                             (lambda (name) (plum/install-one plan name))
                             #:progress print!)))))
  #:inline-output #t)

(define-typed-command! "plum-list-grammars"
  "Show the installed grammars and what else the grammars directory holds."
  (lambda (pane)
    (let* ((in-use (installed-grammar-folders))
           (leftovers (plum/grammar-leftovers in-use)))
      (call! "stdlib/show-report!" pane "plum-grammars"
        (list (call! "stdlib/report-section" "Installed grammars" (map car in-use) #f)
              (call! "stdlib/report-section" "Not in use" leftovers
                "remove with :plum-cleanup-grammars")
              (call! "stdlib/report-section" "Left by stopped installs" (plum/staging-leftovers)
                "clear with :plum-cleanup-grammars"))))))

(define-typed-command! "plum-cleanup-grammars"
  "Delete everything in the grammars directory but the current install of each language HUME knows, and what stopped installs and load checks left."
  (lambda ()
    (let ((paths (append (map (lambda (entry) (path-join (grammars-dir) entry))
                              (plum/grammar-leftovers (installed-grammar-folders)))
                         (map (lambda (entry) (path-join (plum/grammar-staging-dir) entry))
                              (plum/staging-leftovers)))))
      (if (null? paths)
          (log! 'info "PLUM: no grammar files to remove")
          (plum/batch-run! "cleared" paths
            (lambda (path)
              (if (is-dir? path)
                  (call! "stdlib/delete-dir!" path)
                  (call! "stdlib/delete-file!" path))))))))
