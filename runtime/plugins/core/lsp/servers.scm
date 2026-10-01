;;; core:lsp/servers.scm — LSP server install pipeline: download, verify,
;;; unpack, receipt, uninstall, catalog listing. Registration lives in
;;; registration.scm, required below. See docs/servers.md.

(require "registration.scm")

;; ── Source registry ──────────────────────────────────────────────────────────

(define *lsp-sources* (hash))

(define (lsp/declare-source! entry)
  (set! *lsp-sources* (hash-insert *lsp-sources* (car entry) (cdr entry))))

(for-each lsp/declare-source!
  (call-with-input-file
    (path-join (runtime-dir) "scheme" "lsp-sources.scm")
    read))

(define *lsp-lang->server* (hash))

(for-each
  (lambda (name)
    (let* ((fields (hash-ref (lsp/servers-catalog) name))
           (langs  (cdr (lsp/field fields 'languages))))
      (for-each
        (lambda (lang-entry)
          (set! *lsp-lang->server* (hash-insert *lsp-lang->server* (car lang-entry) name)))
        langs)))
  (hash-keys->list (lsp/servers-catalog)))

(define *lsp-hinted-languages* (hash))

;; ── Receipts (write side) ────────────────────────────────────────────────────

(define (lsp/scheme-quote s)
  (string-append "\"" (string-replace (string-replace s "\\" "\\\\") "\"" "\\\"") "\""))

(define (lsp/write-receipt! name version bin)
  (call! "stdlib/write-file!" (lsp/receipt-path name)
    (string-append "((name . " (lsp/scheme-quote name) ")"
                   " (version . " (lsp/scheme-quote version) ")"
                   " (bin . " (lsp/scheme-quote bin) "))")))

(define (lsp/verify-sha256! path expected)
  (let* ((expected-hex (string-downcase
                          (if (starts-with? expected "sha256:")
                              (substring expected 7 (string-length expected))
                              expected)))
         (actual (string-downcase (sha256-file path))))
    (unless (equal? actual expected-hex)
      (call! "stdlib/delete-file!" path)
      (error (string-append "lsp/verify-sha256!: sha256 mismatch for '" path
                            "': expected " expected-hex ", got " actual)))))

;; ── Asset format + installability ─────────────────────────────────────────────

(define *lsp-tar-suffixes* '(".tar.gz" ".tgz" ".tar.xz" ".txz" ".tar.bz2"))

(define (lsp/ends-with-any? s suffixes)
  (if (call! "stdlib/find" (lambda (suffix) (ends-with? s suffix)) suffixes) #t #f))

;; A download is a bare executable when it carries no archive extension; the
;; sync guarantees its bin path equals the asset name in that case.
(define (lsp/asset-format asset-file bin)
  (cond ((lsp/ends-with-any? asset-file *lsp-tar-suffixes*) 'tar)
        ((ends-with? asset-file ".zip") 'zip)
        ((ends-with? asset-file ".gz") 'gz)
        ((equal? asset-file bin) 'raw)
        (else (error (string-append "lsp/asset-format: unsupported asset format: " asset-file)))))

;; The package manager each toolchain-installed kind needs on $PATH.
(define (lsp/toolchain-tool kind)
  (cond ((equal? kind 'npm) "npm")
        ((equal? kind 'cargo) "cargo")
        ((equal? kind 'golang) "go")
        ((equal? kind 'pypi) (if (equal? (hume-target) "windows-x64") "python" "python3"))
        (else #f)))

;; github and generic sources both download one file per platform target.
(define (lsp/download-kind? kind)
  (or (equal? kind 'github) (equal? kind 'generic)))

(define (lsp/find-target targets)
  (let ((want (string->symbol (hume-target))))
    (call! "stdlib/find" (lambda (t) (equal? (list-ref t 0) want)) targets)))

;; (url asset sha bin) for this platform, or #f. A github row is
;; (target asset sha bin) with the url derived from repo and version; a
;; generic row is (target asset url sha bin).
(define (lsp/resolve-download fields)
  (let ((target (lsp/find-target (cdr (lsp/field fields 'targets)))))
    (cond
      ((not target) #f)
      ((equal? (cdr (lsp/field fields 'kind)) 'generic)
       (list (list-ref target 2) (list-ref target 1) (list-ref target 3) (list-ref target 4)))
      (else
       (list (string-append "https://github.com/" (cdr (lsp/field fields 'repo))
                            "/releases/download/" (cdr (lsp/field fields 'version))
                            "/" (list-ref target 1))
             (list-ref target 1) (list-ref target 2) (list-ref target 3))))))

(define (lsp/install-blocker name)
  (cond
    ((not (hume-target)) "unsupported platform")
    ((not (hash-contains? *lsp-sources* name)) "no install source")
    (else
     (let* ((fields (hash-ref *lsp-sources* name))
            (kind   (cdr (lsp/field fields 'kind))))
       (cond
         ((lsp/toolchain-tool kind)
          (let ((tool (lsp/toolchain-tool kind)))
            (if (which tool)
                #f
                (string-append "requires '" tool "' on $PATH, which was not found"))))
         ((not (lsp/download-kind? kind))
          (string-append "not installable (kind " (symbol->string kind) ") in v1"))
         ((not (lsp/resolve-download fields)) "no prebuilt asset for this platform")
         (else #f))))))

;; ── Install pipeline ──────────────────────────────────────────────────────────

;; Linux's tar shells out to the compressor; macOS and Windows tar link their own.
(define (lsp/tar-compressor-tools asset-file)
  (cond ((not (equal? (hume-target) "linux-x64")) '())
        ((lsp/ends-with-any? asset-file '(".tar.xz" ".txz")) '("xz"))
        ((ends-with? asset-file ".tar.bz2") '("bzip2"))
        (else '())))

(define (lsp/required-tools name)
  (let* ((fields (hash-ref *lsp-sources* name))
         (kind   (cdr (lsp/field fields 'kind))))
    (cond
      ((lsp/toolchain-tool kind) (list (lsp/toolchain-tool kind)))
      (else
       (let* ((download (lsp/resolve-download fields))
              (asset    (list-ref download 1))
              (fmt      (lsp/asset-format asset (list-ref download 3))))
         (append
           (cond
             ((equal? fmt 'zip) (list (if (equal? (hume-target) "windows-x64") "tar" "unzip")))
             ((equal? fmt 'tar) (cons "tar" (lsp/tar-compressor-tools asset)))
             ((equal? fmt 'gz) '("gzip"))
             (else '()))
           '("curl")))))))

(define (lsp/preflight! name)
  (for-each
    (lambda (tool)
      (unless (which tool)
        (error (string-append "lsp/install-server!: " name " requires '" tool
                              "' on $PATH, which was not found"))))
    (lsp/required-tools name)))

(define (lsp/install-download! name fields dir)
  (let* ((download (lsp/resolve-download fields))
         (url      (list-ref download 0))
         (asset    (list-ref download 1))
         (sha      (list-ref download 2))
         (bin      (list-ref download 3))
         (fmt      (lsp/asset-format asset bin))
         (archive  (path-join dir asset)))
    (create-directory! dir)
    (run-inline-output! "curl" (list "-fsSL" "-o" archive "--" url))
    (lsp/verify-sha256! archive sha)
    (if (equal? fmt 'raw)
        (mark-executable! archive)
        (begin
          (cond
            ((equal? fmt 'gz) (unpack-gz! archive (path-join dir bin)))
            ((equal? fmt 'zip) (unpack-zip! archive dir bin))
            ((equal? fmt 'tar) (unpack-tar! archive dir bin)))
          (call! "stdlib/delete-file!" archive)))
    (unless (path-exists? (path-join dir bin))
      (error (string-append "lsp/install-download!: " name
                            ": expected binary not found after unpack: " bin)))
    bin))

(define (lsp/install-npm! name fields dir)
  (let* ((packages (cdr (lsp/field fields 'packages)))
         (bin      (cdr (lsp/field fields 'bin)))
         (windows? (equal? (hume-target) "windows-x64"))
         (bin-rel  (string-append "node_modules/.bin/" bin (if windows? ".cmd" ""))))
    (run-inline-output! (if windows? "npm.cmd" "npm")
                        (append (list "install" "--ignore-scripts" "--prefix" dir "--") packages))
    (unless (path-exists? (path-join dir bin-rel))
      (error (string-append "lsp/install-npm!: " name
                            ": expected binary not found after npm install: " bin-rel)))
    bin-rel))

;; Path of `bin` under `bin-dir` (relative to the server dir); errors when the
;; toolchain left no such file.
(define (lsp/managed-bin! who name dir bin-dir bin)
  (let ((bin-rel (string-append bin-dir "/" bin (if (equal? (hume-target) "windows-x64") ".exe" ""))))
    (unless (path-exists? (path-join dir bin-rel))
      (error (string-append "lsp/install-" who "!: " name
                            ": expected binary not found after " who " install: " bin-rel)))
    bin-rel))

(define (lsp/install-cargo! name fields dir)
  (let ((crate   (cdr (lsp/field fields 'crate)))
        (version (cdr (lsp/field fields 'version))))
    (run-inline-output! "cargo"
                        (list "install" "--locked" "--root" dir "--"
                              (string-append crate "@" version)))
    (lsp/managed-bin! "cargo" name dir "bin" (cdr (lsp/field fields 'bin)))))

(define (lsp/install-golang! name fields dir)
  (let ((module  (cdr (lsp/field fields 'module)))
        (version (cdr (lsp/field fields 'version))))
    (run-inline-output! "go"
                        (list "install" "--" (string-append module "@" version))
                        #:env (list (cons "GOBIN" (path-join dir "bin"))))
    (lsp/managed-bin! "go" name dir "bin" (cdr (lsp/field fields 'bin)))))

(define (lsp/install-pypi! name fields dir)
  (let* ((package     (cdr (lsp/field fields 'package)))
         (extras      (cdr (lsp/field fields 'extras)))
         (version     (cdr (lsp/field fields 'version)))
         (windows?    (equal? (hume-target) "windows-x64"))
         (venv        (path-join dir "venv"))
         (venv-bin    (if windows? "venv/Scripts" "venv/bin"))
         (requirement (string-append package
                                     (if (null? extras)
                                         ""
                                         (string-append "[" (string-join extras ",") "]"))
                                     "==" version)))
    (run-inline-output! (lsp/toolchain-tool 'pypi) (list "-m" "venv" venv))
    (run-inline-output! (path-join dir venv-bin (if windows? "python.exe" "python"))
                        (list "-m" "pip" "install" "--disable-pip-version-check"
                              "--" requirement))
    (lsp/managed-bin! "pip" name dir venv-bin (cdr (lsp/field fields 'bin)))))

(define (lsp/install-server! name)
  (let ((blocker (lsp/install-blocker name)))
    (when blocker
      (error (string-append "lsp/install-server!: " name ": " blocker))))
  (lsp/preflight! name)
  (let* ((server-fields (hash-ref (lsp/servers-catalog) name))
         (source-fields (hash-ref *lsp-sources* name))
         (kind          (cdr (lsp/field source-fields 'kind)))
         (dir           (lsp/server-dir name)))
    (for-each (lambda (lang-entry) (unregister-lsp-server! (car lang-entry)))
              (cdr (lsp/field server-fields 'languages)))
    (call! "stdlib/delete-dir!" dir)
    (let ((bin-rel (cond
                     ((lsp/download-kind? kind) (lsp/install-download! name source-fields dir))
                     ((equal? kind 'cargo)  (lsp/install-cargo! name source-fields dir))
                     ((equal? kind 'golang) (lsp/install-golang! name source-fields dir))
                     ((equal? kind 'pypi)   (lsp/install-pypi! name source-fields dir))
                     (else                  (lsp/install-npm! name source-fields dir)))))
      (lsp/write-receipt! name (cdr (lsp/field source-fields 'version)) bin-rel)
      (let ((cmd (cdr (lsp/field server-fields 'command))))
        (when (which cmd)
          (log! 'info (string-append "LSP: " cmd " is also on $PATH — the managed install at "
                                     (path-join dir bin-rel) " takes precedence")))))))

;; ── Commands ──────────────────────────────────────────────────────────────────

(define (lsp/with-install-lock! what thunk)
  (let ((acquired?
          (with-handler
            (lambda (err) (log! 'error (string-append "LSP: " (to-string err))) #f)
            (begin (acquire-install-lock!) #t))))
    (and acquired?
         (with-handler
           (lambda (err)
             (release-install-lock!)
             (log! 'error (string-append "LSP: " what " failed: " (to-string err)))
             #f)
           (begin (thunk) (release-install-lock!) #t)))))

(define (lsp/lsp-install-or-report! name)
  (let* ((receipt (lsp/read-receipt name))
         (source  (if (hash-contains? *lsp-sources* name)
                      (hash-ref *lsp-sources* name)
                      #f)))
    (if (and receipt source
             (equal? (lsp/receipt-version receipt) (cdr (lsp/field source 'version))))
        (begin
          (log! 'info (string-append "LSP: " name " already installed (v"
                                     (lsp/receipt-version receipt) ") — up to date"))
          (lsp/register-installed-servers!))
        (let ((had-dir? (path-exists? (lsp/server-dir name))))
          (if (lsp/with-install-lock! (string-append "install " name)
                (lambda ()
                  (log! 'info (string-append "LSP: installing " name "..."))
                  (lsp/install-server! name)))
              (lsp/register-installed-servers!)
              (when had-dir?
                (log! 'info "LSP: if the server was running it has now been shut down — run :lsp-install again")))))))

(register-completion-source! "lsp:languages"
  (lambda (id input cursor)
    (completion-emit! id (hash-keys->list *lsp-lang->server*)))
  #:target 'minibuf #:match 'string)

(define-typed-command! "lsp-install"
  "Download and verify the language server for a language (default: the current buffer's language), then register it."
  (lambda (pane arg)
    (let ((lang (call! "stdlib/resolve-lang-arg" pane "lsp-install" arg)))
      (cond
        ((not lang) (begin))
        ((not (hash-contains? *lsp-lang->server* lang))
         (log! 'info (string-append "lsp-install: no language server is seeded for \"" lang "\"")))
        (else
         (lsp/lsp-install-or-report! (hash-ref *lsp-lang->server* lang))))))
  #:inline-output #t #:complete "lsp:languages")

(register-completion-source! "lsp:servers"
  (lambda (id input cursor)
    (let ((sdir (lsp/servers-dir)))
      (completion-emit! id
        (if (path-exists? sdir)
            (call! "stdlib/list-subdirs" sdir)
            '()))))
  #:target 'minibuf #:match 'string)

(define-typed-command! "lsp-uninstall"
  "Shut down and remove an installed language server by name."
  (lambda (pane arg)
    (cond
      ((not (string? arg))
       (log! 'info "lsp-uninstall: requires a server name, e.g. :lsp-uninstall rust-analyzer"))
      ((not (eq? #t (call! "stdlib/safe-path-segment?" arg)))
       (log! 'warn (string-append "lsp-uninstall: invalid server name: " arg)))
      (else
        (let* ((name arg)
               (dir  (lsp/server-dir name)))
          (when (hash-contains? (lsp/servers-catalog) name)
            (for-each (lambda (lang-entry) (unregister-lsp-server! (car lang-entry)))
                      (cdr (lsp/field (hash-ref (lsp/servers-catalog) name) 'languages))))
          (if (path-exists? dir)
              (begin
                (log! 'info (string-append "LSP: shutting down and removing " name "..."))
                (after! 0 (lambda ()
                           (when (lsp/with-install-lock! (string-append "uninstall " name)
                                   (lambda () (call! "stdlib/delete-dir!" dir)))
                             (log! 'info (string-append "LSP: removed " name))))))
              (log! 'info (string-append "LSP: nothing to uninstall for " name)))))))
  #:complete "lsp:servers")

(define-typed-command! "lsp-servers"
  "Log the LSP server catalog: languages, seeded version, and install status."
  (lambda ()
    (for-each
      (lambda (name)
        (let* ((receipt (lsp/read-receipt name))
               (source  (if (hash-contains? *lsp-sources* name)
                            (hash-ref *lsp-sources* name)
                            #f))
               (langs   (map car (cdr (lsp/field (hash-ref (lsp/servers-catalog) name) 'languages))))
               (status
                 (cond
                   (receipt
                    (let ((installed (lsp/receipt-version receipt))
                          (seeded    (if source (cdr (lsp/field source 'version)) #f)))
                      (if (and seeded (not (equal? installed seeded)))
                          (string-append "installed v" installed " — update available (v" seeded ")")
                          (string-append "installed v" installed))))
                   (else
                    (let ((blocker (lsp/install-blocker name)))
                      (if blocker blocker "not installed"))))))
          (displayln (string-append name " [" (string-join langs ", ") "]: " status))))
      (hash-keys->list (lsp/servers-catalog)))
    (log! 'info (string-append "LSP: " (number->string (length (hash-keys->list (lsp/servers-catalog))))
                               " seeded servers")))
  #:inline-output #t)

;; ── Discovery hint ────────────────────────────────────────────────────────────

(register-hook! 'on-language-set
  (lambda (pane lang)
    (when (and (not (equal? lang "")) (not (hash-contains? *lsp-hinted-languages* lang)))
      (set! *lsp-hinted-languages* (hash-insert *lsp-hinted-languages* lang #t))
      (when (hash-contains? *lsp-lang->server* lang)
        (let* ((name    (hash-ref *lsp-lang->server* lang))
               (blocker (lsp/install-blocker name)))
          (when (and (not blocker)
                     (not (lsp-registered-for-language? lang))
                     (not (lsp/read-receipt name)))
            (log! 'warn (string-append "LSP: language server '" name
                                       "' is available for " lang " — run :lsp-install"))))))))
