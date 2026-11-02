;;; core:plum/grammars.scm — the grammar install pipeline; see docs/grammars.md.

(require "lib/helpers.scm")
(require-builtin steel/time)

;; ── Grammar source ────────────────────────────────────────────────────────────
;; See docs/grammars.md's "Grammar source".

(define *plum-grammar-tag*
  (read-data-file (path-join (plugin-dir) "data" "treesitter-hume-pin.scm")))

;;; The folder of grammar archives `#:config 'grammar-source` names, or #f for none.
(define *plum-grammar-folder*
  (let ((cfg (plugin-config)))
    (and (hash-contains? cfg 'grammar-source)
         (let ((folder (call! "stdlib/config-string" "core:plum" cfg 'grammar-source "")))
           (unless (call! "stdlib/path-absolute?" folder)
             (error (string-append "'grammar-source must be an absolute path: " folder)))
           folder))))

(define (plum/release-url file)
  (string-append "https://github.com/cvlmtg/treesitter.hume/releases/download/"
                 *plum-grammar-tag* "/" file))

(define (plum/bundle-file ext)
  (string-append "grammars-" (grammar-platform) "." ext))

(define (plum/grammar-staging-dir)
  (path-join (plum/staging-dir) "grammars"))

(define (plum/archive-file name)
  (string-append name ".tar.gz"))

;;; Calls `body` on the path of `file` in the grammar staging directory and returns its
;;; result, deleting the file once `body` returns or raises.
(define (plum/with-staged-file! file body)
  (let ((path (path-join (plum/grammar-staging-dir) file)))
    (create-directory! (plum/grammar-staging-dir))
    (dynamic-wind (lambda () #t)
                  (lambda () (body path))
                  (lambda () (call! "stdlib/delete-file!" path)))))

;; ── Index ─────────────────────────────────────────────────────────────────────
;; See docs/grammars.md's "Grammar source".

(define (plum/index-error reason)
  (error (string-append "the grammar index " reason)))

(define (plum/parse-member line)
  (let* ((fields (split-whitespace line))
         (four (= (length fields) 4))
         (offset (and four (string->number (list-ref fields 1))))
         (size (and four (string->number (list-ref fields 2)))))
    (unless (and (integer? offset) (integer? size) (>= offset 0) (> size 0))
      (plum/index-error (string-append "has a malformed line: " line)))
    (list (car fields) offset size (list-ref fields 3))))

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
        (error (string-append "cannot fetch the treesitter.hume " *plum-grammar-tag*
                              " grammar index for " (grammar-platform) ": "
                              (error-text err))))
      (plum/with-staged-file! "index"
        (lambda (path)
          (print! (plum/msg "fetching the treesitter.hume " *plum-grammar-tag* " grammar index"))
          (call! "stdlib/download!" (plum/release-url (plum/bundle-file "index")) path)
          (call-with-input-file path read-port-to-string))))))

;;; Calls `use` on `name`'s release archive, downloaded and digest-checked, then deletes it.
(define (plum/with-release-member! name member use)
  (plum/with-staged-file! (plum/archive-file name)
    (lambda (archive)
      (call! "stdlib/download!" (plum/release-url (plum/bundle-file "tar")) archive
             (list-ref member 1) (list-ref member 2))
      (call! "stdlib/verify-sha256!" archive (list-ref member 3))
      (use archive))))

;;; The configured folder, or #f when none is configured. See docs/grammars.md's "Grammar source".
(define (plum/checked-folder)
  (let* ((folder *plum-grammar-folder*)
         (fail (lambda (reason)
                 (error (string-append "'grammar-source " folder " " reason)))))
    (when folder
      (unless (is-dir? folder)
        (fail "is not a directory"))
      (for-each
        (lambda (managed)
          (unless (first-path-outside managed (list folder))
            (fail (string-append "is inside " managed ", which PLUM manages"))))
        (list (plum/staging-dir) (grammars-dir)))
      (let ((archives (map plum/archive-file (bundled-languages))))
        (unless (call! "stdlib/find" (lambda (entry) (member entry archives))
                       (call! "stdlib/list-entries" folder))
          (fail "holds no grammar archive"))))
    folder))

;;; See docs/grammars.md's "Grammar source".
(define (plum/source-lookup)
  (let ((folder (plum/checked-folder))
        (release #f))
    (lambda (name)
      (let ((archive (and folder (path-join folder (plum/archive-file name)))))
        (if (and archive (path-exists? archive))
            (lambda (use) (use archive))
            (begin
              (unless release
                (set! release
                  (with-handler
                    (lambda (err)
                      (if folder
                          (error (string-append "no " name " grammar in " folder "; "
                                                (error-text err)))
                          (raise-error err)))
                    (plum/read-release-index))))
              (let ((member (assoc name release)))
                (and member
                     (lambda (use) (plum/with-release-member! name member use))))))))))

;; ── Install pipeline ──────────────────────────────────────────────────────────
;; See docs/grammars.md's "Grammar install pipeline".

(define (plum/check-unpacked! name dir)
  (let* ((manifest (grammar-manifest dir))
         (field (lambda (key) (grammar-manifest-ref manifest key)))
         (symbol (field 'symbol))
         (fail (lambda (reason) (error (string-append "the " name " grammar " reason)))))
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
      (log! 'warning (plum/msg "the " name " grammar is from treesitter.hume "
                               (to-string (field 'tag)) "; HUME pins " *plum-grammar-tag*)))))

;;; Hash: name → (dep-name ...). See docs/grammars.md's "Grammar dependencies".
(define *plum-grammar-deps*
  (hash "markdown" (list "markdown.inline")))

;;; A name for the next install folder after `installs`, a `grammar-installs` list.
(define (plum/new-install-name installs)
  (let ((now (current-milliseconds)))
    (number->string
      (if (null? installs)
          now
          (max now (+ 1 (string->number (last installs))))))))

;;; Deletes the directory `path`, reporting a failure; see docs/grammars.md's "Grammar install pipeline".
(define (plum/delete-reporting! path)
  (with-handler
    (lambda (err)
      (let ((failure (plum/msg "could not delete " path ": " (error-text err))))
        (if (equal? (current-os!) "windows")
            (log! 'warning (string-append failure ". A grammar library HUME has loaded cannot be "
                                          "deleted on Windows: restart HUME and run "
                                          ":plum-cleanup-grammars"))
            (log! 'error failure))))
    (call! "stdlib/delete-dir!" path)))

;;; Installs `name` from `source`, its entry in a `plum/served-plan` result.
;;; See docs/grammars.md's "Grammar install pipeline".
(define (plum/install-one name source)
  (let* ((older (grammar-installs name))
         (folder (path-join (grammar-dir name) (plum/new-install-name older))))
    (plum/staged-install! (path-join (plum/grammar-staging-dir) name) folder
      (lambda (stage)
        (source (lambda (archive) (call! "stdlib/extract-archive!" "tar" archive stage)))
        (plum/check-unpacked! name stage)))
    (with-handler
      (lambda (err)
        (plum/delete-reporting! folder)
        (when (null? (call! "stdlib/list-entries" (grammar-dir name)))
          (call! "stdlib/delete-dir!" (grammar-dir name)))
        (raise-error err))
      (check-grammar! name folder)
      (mark-grammar-checked! folder))
    (register-installed-grammar! name folder)
    (for-each (lambda (old) (plum/delete-reporting! (path-join (grammar-dir name) old)))
              older)))

;; ── Grammar discovery ─────────────────────────────────────────────────────────

(define (plum/not-installed? name)
  (not (grammar-installed? name)))

(define (plum/grammar-deps name)
  (or (hash-try-get *plum-grammar-deps* name) '()))

;;; `plan`, newest first, with `name` after the not-installed dependencies it needs.
(define (plum/plan-add name plan)
  (if (member name plan)
      plan
      (cons name (foldl plum/plan-add plan
                        (filter plum/not-installed? (plum/grammar-deps name))))))

;;; `((name . source) ...)` in install order; see docs/grammars.md's "Grammar install pipeline".
(define (plum/served-plan names)
  (let* ((lookup (plum/source-lookup))
         (plan (map (lambda (name) (cons name (lookup name)))
                    (reverse (foldl plum/plan-add '() names))))
         (absent (map car (filter (lambda (entry) (not (cdr entry))) plan))))
    (unless (null? absent)
      (error (string-append "no " (string-join absent ", ") " grammar in "
                            (if *plum-grammar-folder*
                                (string-append *plum-grammar-folder* " or ")
                                "")
                            "treesitter.hume " *plum-grammar-tag* " for " (grammar-platform))))
    plan))

;;; Installs `plan`, a `plum/served-plan` result, skipping a grammar whose dependency failed,
;;; and returns how many installed. See docs/grammars.md's "Grammar install pipeline".
(define (plum/install-plan! plan)
  (plum/batch-run! "installed grammar" (map car plan)
    (lambda (name) (plum/install-one name (cdr (assoc name plan))))
    #:skip (lambda (name failed)
             (let ((dep (call! "stdlib/find" (lambda (dep) (member dep failed))
                               (plum/grammar-deps name))))
               (and dep (string-append "skipped, it needs " dep ", which did not install"))))
    #:progress "installing grammar"))

(define (plum/entries-if-present dir)
  (if (path-exists? dir) (call! "stdlib/list-entries" dir) '()))

;;; Paths relative to `<data>/grammars/`; see docs/grammars.md's "Leftovers".
(define (plum/grammar-leftovers in-use)
  (call! "stdlib/concat"
         (map (lambda (entry)
                (let ((installed (assoc entry in-use)))
                  (if installed
                      (let ((current (file-name (cdr installed))))
                        (map (lambda (inner) (path-join entry inner))
                             (filter (lambda (inner) (not (equal? inner current)))
                                     (call! "stdlib/list-entries" (grammar-dir entry)))))
                      (list entry))))
              (plum/entries-if-present (grammars-dir)))))

;;; See docs/grammars.md's "Leftovers".
(define (plum/staging-leftovers)
  (plum/entries-if-present (plum/grammar-staging-dir)))

;;; A string argument wins; otherwise falls back to the current buffer's language.
(define (plum/resolve-grammar-arg pane arg)
  (let ((name (call! "stdlib/resolve-lang-arg" pane "plum-install-grammar" arg)))
    (cond ((not name) #f)
          ((not (member name (bundled-languages)))
           (log! 'info (plum/msg "no language \"" name "\"; Tab after :plum-install-grammar lists the languages"))
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
    (let ((name (plum/resolve-grammar-arg pane arg)))
      (when name
        (plum/report-failure! "install"
          (lambda ()
            (let ((plan (plum/served-plan (list name))))
              (when (= (plum/install-plan! plan) (length plan))
                (print! (plum/msg "installed grammar for " name)))))))))
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
          (log! 'info (plum/msg "all requested grammars are installed"))
          (plum/install-plan! (plum/served-plan missing)))))
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
  "Delete everything in the grammars directory but the current install of each language HUME knows, and what stopped installs left."
  (lambda ()
    (let ((paths (append (map (lambda (entry) (path-join (grammars-dir) entry))
                              (plum/grammar-leftovers (installed-grammar-folders)))
                         (map (lambda (entry) (path-join (plum/grammar-staging-dir) entry))
                              (plum/staging-leftovers)))))
      (plum/batch-run! "cleared" paths
        (lambda (path)
          (if (is-dir? path)
              (call! "stdlib/delete-dir!" path)
              (call! "stdlib/delete-file!" path)))
        #:none "no grammar files to remove"))))
