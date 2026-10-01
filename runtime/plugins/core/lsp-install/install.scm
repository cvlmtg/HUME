;;; core:lsp-install/install.scm — see README.md.

(require "catalog.scm")
(require "receipts.scm")
(require "platform.scm")
(require "register.scm")
(require "sha256.scm")
(require "unpack.scm")

(provide lsp-install/install-server! lsp-install/install-blocker)

(define (lsp-install/kind-env-dirs kind)
  (if (equal? kind 'gem)
      '(("GEM_HOME" . ".") ("GEM_PATH" . "."))
      '()))

(define (lsp-install/verify-sha256! path expected)
  (let* ((expected-hex (string-downcase
                          (if (starts-with? expected "sha256:")
                              (substring expected 7 (string-length expected))
                              expected)))
         (actual (lsp-install/sha256-file path)))
    (unless (equal? actual expected-hex)
      (call! "stdlib/delete-file!" path)
      (error (string-append "lsp-install/verify-sha256!: sha256 mismatch for '" path
                            "': expected " expected-hex ", got " actual)))))

;; ── Asset format + installability ─────────────────────────────────────────────

(define *lsp-install-tar-suffixes* '(".tar.gz" ".tgz" ".tar.xz" ".txz" ".tar.bz2"))

(define (lsp-install/ends-with-any? s suffixes)
  (call! "stdlib/find" (lambda (suffix) (ends-with? s suffix)) suffixes))

(define (lsp-install/asset-format asset-file bin)
  (cond ((lsp-install/ends-with-any? asset-file *lsp-install-tar-suffixes*) 'tar)
        ((ends-with? asset-file ".zip") 'zip)
        ((ends-with? asset-file ".gz") 'gz)
        ((equal? asset-file bin) 'raw)
        (else (error (string-append "lsp-install/asset-format: unsupported asset format: " asset-file)))))

(define (lsp-install/toolchain-tool kind)
  (cond ((equal? kind 'npm) "npm")
        ((equal? kind 'cargo) "cargo")
        ((equal? kind 'golang) "go")
        ((equal? kind 'pypi) (if lsp-install/windows? "python" "python3"))
        ((equal? kind 'gem) "gem")
        ((equal? kind 'nuget) "dotnet")
        (else #f)))

(define (lsp-install/platform-supported? fields)
  (let ((platforms (assoc 'platforms fields)))
    (or (not platforms)
        (member (string->symbol lsp-install/target) (cdr platforms)))))

(define (lsp-install/download-kind? kind)
  (or (equal? kind 'github) (equal? kind 'generic)))

(define (lsp-install/find-target targets)
  (let ((want (string->symbol lsp-install/target)))
    (call! "stdlib/find" (lambda (t) (equal? (list-ref t 0) want)) targets)))

(define (lsp-install/resolve-download fields)
  (let ((target (lsp-install/find-target (lsp-install/ref fields 'targets))))
    (cond
      ((not target) #f)
      ((equal? (lsp-install/ref fields 'kind) 'generic)
       (list (list-ref target 2) (list-ref target 1) (list-ref target 3) (list-ref target 4)))
      (else
       (list (string-append "https://github.com/" (lsp-install/ref fields 'repo)
                            "/releases/download/" (lsp-install/ref fields 'version)
                            "/" (list-ref target 1))
             (list-ref target 1) (list-ref target 2) (list-ref target 3))))))

(define (lsp-install/install-blocker name)
  (cond
    ((not lsp-install/target) "unsupported platform")
    ((not (lsp-install/source name)) "no install source")
    (else
     (let* ((fields (lsp-install/source name))
            (kind   (lsp-install/ref fields 'kind))
            (tool   (lsp-install/toolchain-tool kind)))
       (cond
         ((not (lsp-install/platform-supported? fields)) "not supported on this platform")
         (tool
          (if (which tool)
              #f
              (string-append "requires '" tool "' on $PATH, which was not found")))
         ((not (lsp-install/download-kind? kind))
          (string-append "not installable (kind " (symbol->string kind) ") in v1"))
         ((not (lsp-install/resolve-download fields)) "no prebuilt asset for this platform")
         (else #f))))))

;; ── Install pipeline ──────────────────────────────────────────────────────────

(define (lsp-install/tar-compressor-tools asset-file)
  (cond ((not (equal? lsp-install/target "linux-x64")) '())
        ((lsp-install/ends-with-any? asset-file '(".tar.xz" ".txz")) '("xz"))
        ((ends-with? asset-file ".tar.bz2") '("bzip2"))
        (else '())))

(define (lsp-install/required-tools name)
  (let* ((fields (lsp-install/source name))
         (kind   (lsp-install/ref fields 'kind))
         (tool   (lsp-install/toolchain-tool kind)))
    (cond
      (tool (list tool))
      (else
       (let* ((download (lsp-install/resolve-download fields))
              (asset    (list-ref download 1))
              (fmt      (lsp-install/asset-format asset (list-ref download 3))))
         (append
           (cond
             ((equal? fmt 'zip) (list (lsp-install/unpack-tool 'zip)))
             ((equal? fmt 'tar) (cons "tar" (lsp-install/tar-compressor-tools asset)))
             ((equal? fmt 'gz) '("gzip"))
             (else '()))
           '("curl")))))))

(define (lsp-install/preflight! name)
  (for-each
    (lambda (tool)
      (unless (which tool)
        (error (string-append "lsp-install/install-server!: " name " requires '" tool
                              "' on $PATH, which was not found"))))
    (lsp-install/required-tools name)))

(define (lsp-install/install-download! name fields dir)
  (let* ((download (lsp-install/resolve-download fields))
         (url      (list-ref download 0))
         (asset    (list-ref download 1))
         (sha      (list-ref download 2))
         (bin      (list-ref download 3))
         (fmt      (lsp-install/asset-format asset bin))
         (archive  (path-join dir asset)))
    (create-directory! dir)
    (run-inline-output! "curl" (list "-fsSL" "-o" archive "--" url))
    (lsp-install/verify-sha256! archive sha)
    (if (equal? fmt 'raw)
        (lsp-install/mark-executable! (list archive))
        (begin
          (cond
            ((equal? fmt 'gz) (lsp-install/unpack-gz! archive (path-join dir bin)))
            ((equal? fmt 'zip) (lsp-install/unpack-archive! 'zip archive dir))
            ((equal? fmt 'tar) (lsp-install/unpack-archive! 'tar archive dir)))
          (call! "stdlib/delete-file!" archive)))
    (unless (path-exists? (path-join dir bin))
      (error (string-append "lsp-install/install-download!: " name
                            ": expected binary not found after unpack: " bin)))
    bin))

(define (lsp-install/managed-bin! who name dir bin-dir bin windows-suffix)
  (let ((bin-rel (string-append bin-dir "/" bin
                                (if lsp-install/windows? windows-suffix ""))))
    (unless (path-exists? (path-join dir bin-rel))
      (error (string-append "lsp-install/install-" who "!: " name
                            ": expected binary not found after " who " install: " bin-rel)))
    bin-rel))

(define (lsp-install/install-npm! name fields dir)
  (run-inline-output! (if lsp-install/windows? "npm.cmd" "npm")
                      (append (list "install" "--ignore-scripts" "--prefix" dir "--")
                              (lsp-install/ref fields 'packages)))
  (lsp-install/managed-bin! "npm" name dir "node_modules/.bin" (lsp-install/ref fields 'bin) ".cmd"))

(define (lsp-install/install-cargo! name fields dir)
  (let ((crate   (lsp-install/ref fields 'crate))
        (version (lsp-install/ref fields 'version)))
    (run-inline-output! "cargo"
                        (list "install" "--locked" "--root" dir "--"
                              (string-append crate "@" version)))
    (lsp-install/managed-bin! "cargo" name dir "bin" (lsp-install/ref fields 'bin) ".exe")))

(define (lsp-install/install-golang! name fields dir)
  (let ((module  (lsp-install/ref fields 'module))
        (version (lsp-install/ref fields 'version)))
    (run-inline-output! "go"
                        (list "install" "--" (string-append module "@" version))
                        #:env (list (cons "GOBIN" (path-join dir "bin"))))
    (lsp-install/managed-bin! "go" name dir "bin" (lsp-install/ref fields 'bin) ".exe")))

(define (lsp-install/install-pypi! name fields dir)
  (let* ((package     (lsp-install/ref fields 'package))
         (extras      (lsp-install/ref fields 'extras))
         (version     (lsp-install/ref fields 'version))
         (windows?    lsp-install/windows?)
         (venv        (path-join dir "venv"))
         (venv-bin    (if windows? "venv/Scripts" "venv/bin"))
         (requirement (string-append package
                                     (if (null? extras)
                                         ""
                                         (string-append "[" (string-join extras ",") "]"))
                                     "==" version)))
    (run-inline-output! (lsp-install/toolchain-tool 'pypi) (list "-m" "venv" venv))
    (run-inline-output! (path-join dir venv-bin (if windows? "python.exe" "python"))
                        (list "-m" "pip" "install" "--disable-pip-version-check"
                              "--" requirement))
    (lsp-install/managed-bin! "pip" name dir venv-bin (lsp-install/ref fields 'bin) ".exe")))

(define (lsp-install/install-nuget! name fields dir)
  (run-inline-output! "dotnet"
                      (list "tool" "install" (lsp-install/ref fields 'package)
                            "--tool-path" (path-join dir "bin")
                            "--version" (lsp-install/ref fields 'version)))
  (lsp-install/managed-bin! "dotnet" name dir "bin" (lsp-install/ref fields 'bin) ".exe"))

(define (lsp-install/install-gem! name fields dir)
  (run-inline-output! (if lsp-install/windows? "gem.cmd" "gem")
                      (append (list "install" "--no-document" "--install-dir" dir
                                    "--bindir" (path-join dir "bin"))
                              (lsp-install/ref fields 'packages)))
  (lsp-install/managed-bin! "gem" name dir "bin" (lsp-install/ref fields 'bin) ".bat"))

(define (lsp-install/install-server! name)
  (let ((blocker (lsp-install/install-blocker name)))
    (when blocker
      (error (string-append "lsp-install/install-server!: " name ": " blocker))))
  (lsp-install/preflight! name)
  (let* ((server-fields (hash-ref lsp-install/servers name))
         (source-fields (lsp-install/source name))
         (kind          (lsp-install/ref source-fields 'kind))
         (dir           (lsp-install/server-dir name)))
    (lsp-install/unregister-server-languages! name)
    (call! "stdlib/delete-dir!" dir)
    (let ((bin-rel (cond
                     ((lsp-install/download-kind? kind) (lsp-install/install-download! name source-fields dir))
                     ((equal? kind 'cargo)  (lsp-install/install-cargo! name source-fields dir))
                     ((equal? kind 'golang) (lsp-install/install-golang! name source-fields dir))
                     ((equal? kind 'pypi)   (lsp-install/install-pypi! name source-fields dir))
                     ((equal? kind 'gem)    (lsp-install/install-gem! name source-fields dir))
                     ((equal? kind 'nuget)  (lsp-install/install-nuget! name source-fields dir))
                     (else                  (lsp-install/install-npm! name source-fields dir)))))
      (lsp-install/write-receipt! name (lsp-install/ref source-fields 'version) bin-rel
                          (lsp-install/kind-env-dirs kind))
      (let ((cmd (lsp-install/ref server-fields 'command)))
        (when (which cmd)
          (log! 'info (string-append "LSP: " cmd " is also on $PATH — the managed install at "
                                     (path-join dir bin-rel) " takes precedence")))))))
