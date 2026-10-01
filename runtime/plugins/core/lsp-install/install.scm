;;; core:lsp-install/install.scm — see README.md.

(require "catalog.scm")
(require "receipts.scm")
(require "platform.scm")
(require "register.scm")
(require "sha256.scm")
(require "unpack.scm")

(provide lsp-install/install-server! lsp-install/install-blocker)

(define lsp-install/python (if lsp-install/windows? "python" "python3"))

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
        (else #f)))

(define (lsp-install/platform-supported? fields)
  (let ((platforms (assoc 'platforms fields)))
    (or (not platforms)
        (member (string->symbol lsp-install/target) (cdr platforms)))))

(define (lsp-install/find-target targets)
  (let ((want (string->symbol lsp-install/target)))
    (call! "stdlib/find" (lambda (t) (equal? (list-ref t 0) want)) targets)))

(define (lsp-install/tar-compressor-tools asset-file)
  (cond ((not (equal? lsp-install/target "linux-x64")) '())
        ((lsp-install/ends-with-any? asset-file '(".tar.xz" ".txz")) '("xz"))
        ((ends-with? asset-file ".tar.bz2") '("bzip2"))
        (else '())))

(define (lsp-install/download-tools fmt asset)
  (append
    (cond ((equal? fmt 'zip) (list (lsp-install/unpack-tool 'zip)))
          ((equal? fmt 'tar) (cons "tar" (lsp-install/tar-compressor-tools asset)))
          ((equal? fmt 'gz) '("gzip"))
          (else '()))
    '("curl")))

;; ── Install pipeline ──────────────────────────────────────────────────────────

(define (lsp-install/install-download! name url asset sha bin fmt dir)
  (let ((archive (path-join dir asset)))
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
    (run-inline-output! lsp-install/python (list "-m" "venv" venv))
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

;; ── Per-kind plans ────────────────────────────────────────────────────────────
;; A plan is the tools a kind needs on $PATH, the env dirs its receipt records,
;; and the procedure that installs into a directory and returns the bin path.

(define (lsp-install/plan tools env-dirs install!)
  (hash 'tools tools 'env-dirs env-dirs 'install! install!))

(define (lsp-install/download-plan name fields)
  (let ((target (lsp-install/find-target (lsp-install/ref fields 'targets))))
    (if (not target)
        "no prebuilt asset for this platform"
        (let* ((asset (list-ref target 1))
               (row   (if (equal? (lsp-install/ref fields 'kind) 'generic)
                          (cdr target)
                          (list asset
                                (string-append "https://github.com/" (lsp-install/ref fields 'repo)
                                               "/releases/download/" (lsp-install/ref fields 'version)
                                               "/" asset)
                                (list-ref target 2)
                                (list-ref target 3))))
               (url   (list-ref row 1))
               (sha   (list-ref row 2))
               (bin   (list-ref row 3))
               (fmt   (lsp-install/asset-format asset bin)))
          (if (not fmt)
              (string-append "unsupported asset format: " asset)
              (lsp-install/plan
                (lsp-install/download-tools fmt asset)
                '()
                (lambda (dir)
                  (lsp-install/install-download! name url asset sha bin fmt dir))))))))

;; Rows: (kind tool env-dirs installer), installer taking (name fields dir).
(define lsp-install/package-kinds
  (list (list 'npm    "npm"              '() lsp-install/install-npm!)
        (list 'cargo  "cargo"            '() lsp-install/install-cargo!)
        (list 'golang "go"               '() lsp-install/install-golang!)
        (list 'pypi   lsp-install/python '() lsp-install/install-pypi!)
        (list 'gem    "gem" '(("GEM_HOME" . ".") ("GEM_PATH" . ".")) lsp-install/install-gem!)
        (list 'nuget  "dotnet"           '() lsp-install/install-nuget!)))

(define (lsp-install/kind-plan name fields)
  (let* ((kind (lsp-install/ref fields 'kind))
         (row  (assoc kind lsp-install/package-kinds)))
    (cond
      ((or (equal? kind 'github) (equal? kind 'generic))
       (lsp-install/download-plan name fields))
      (row
       (lsp-install/plan (list (list-ref row 1)) (list-ref row 2)
                         (lambda (dir) ((list-ref row 3) name fields dir))))
      (else (string-append "not installable (kind " (symbol->string kind) ") in v1")))))

;; The plan for `name`, or a string naming what blocks installing it.
(define (lsp-install/resolve name)
  (let ((fields (lsp-install/source name)))
    (cond
      ((not lsp-install/target) "unsupported platform")
      ((not fields) "no install source")
      ((not (lsp-install/platform-supported? fields)) "not supported on this platform")
      (else
       (let ((plan (lsp-install/kind-plan name fields)))
         (if (string? plan)
             plan
             (let ((missing (call! "stdlib/find" (lambda (tool) (not (which tool)))
                                   (hash-ref plan 'tools))))
               (if missing
                   (string-append "requires '" missing "' on $PATH, which was not found")
                   plan))))))))

(define (lsp-install/install-blocker name)
  (let ((resolved (lsp-install/resolve name)))
    (and (string? resolved) resolved)))

(define (lsp-install/install-server! name)
  (let ((plan (lsp-install/resolve name)))
    (when (string? plan)
      (error (string-append "lsp-install/install-server!: " name ": " plan)))
    (let ((server-fields (hash-ref lsp-install/servers name))
          (dir           (lsp-install/server-dir name)))
      (lsp-install/unregister-server-languages! name)
      (call! "stdlib/delete-dir!" dir)
      (let ((bin-rel ((hash-ref plan 'install!) dir)))
        (lsp-install/write-receipt! name (lsp-install/ref (lsp-install/source name) 'version)
                                    bin-rel (hash-ref plan 'env-dirs))
        (let ((cmd (lsp-install/ref server-fields 'command)))
          (when (which cmd)
            (log! 'info (string-append "LSP: " cmd " is also on $PATH — the managed install at "
                                       (path-join dir bin-rel) " takes precedence"))))))))
