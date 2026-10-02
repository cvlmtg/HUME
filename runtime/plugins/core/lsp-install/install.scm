;;; core:lsp-install/install.scm — see README.md.

(require "catalog.scm")
(require "source-catalog.scm")
(require "receipts.scm")
(require "platform.scm")
(require "blocker.scm")
(require "register.scm")
(require "sha256.scm")
(require "unpack.scm")

(provide lsp-install/install-server!)

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

;; ── Install pipeline ──────────────────────────────────────────────────────────

(define (lsp-install/install-download! name url asset sha bin fmt unpacker dir)
  (let ((archive (path-join dir asset)))
    (create-directory! dir)
    (run-inline-output! "curl" (list "-fsSL" "-o" archive "--" url))
    (lsp-install/verify-sha256! archive sha)
    (if (equal? fmt 'raw)
        (lsp-install/mark-executable! (list archive))
        (begin
          (cond
            ((equal? fmt 'gz) (lsp-install/unpack-gz! archive (path-join dir bin)))
            (else (lsp-install/unpack-archive! unpacker archive dir)))
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

(define (lsp-install/install-npm! name fields row dir)
  (run-inline-output! (if lsp-install/windows? "npm.cmd" "npm")
                      (append (list "install" "--ignore-scripts" "--prefix" dir "--")
                              (lsp-install/ref fields 'packages)))
  (lsp-install/managed-bin! "npm" name dir "node_modules/.bin" (lsp-install/ref fields 'bin) ".cmd"))

(define (lsp-install/install-cargo! name fields row dir)
  (let ((crate   (lsp-install/ref fields 'crate))
        (version (lsp-install/ref fields 'version)))
    (run-inline-output! "cargo"
                        (list "install" "--locked" "--root" dir "--"
                              (string-append crate "@" version)))
    (lsp-install/managed-bin! "cargo" name dir "bin" (lsp-install/ref fields 'bin) ".exe")))

(define (lsp-install/install-golang! name fields row dir)
  (let ((module  (lsp-install/ref fields 'module))
        (version (lsp-install/ref fields 'version)))
    (run-inline-output! "go"
                        (list "install" "--" (string-append module "@" version))
                        #:env (list (cons "GOBIN" (path-join dir "bin"))))
    (lsp-install/managed-bin! "go" name dir "bin" (lsp-install/ref fields 'bin) ".exe")))

(define (lsp-install/install-pypi! name fields row dir)
  (let* ((package     (lsp-install/ref fields 'package))
         (extras      (lsp-install/ref fields 'extras))
         (version     (lsp-install/ref fields 'version))
         (windows?    lsp-install/windows?)
         (python      (car (lsp-install/row-tools row)))
         (venv        (path-join dir "venv"))
         (venv-bin    (if windows? "venv/Scripts" "venv/bin"))
         (requirement (string-append package
                                     (if (null? extras)
                                         ""
                                         (string-append "[" (string-join extras ",") "]"))
                                     "==" version)))
    (run-inline-output! python (list "-m" "venv" venv))
    (run-inline-output! (path-join dir venv-bin (if windows? "python.exe" "python"))
                        (list "-m" "pip" "install" "--disable-pip-version-check"
                              "--" requirement))
    (lsp-install/managed-bin! "pip" name dir venv-bin (lsp-install/ref fields 'bin) ".exe")))

(define (lsp-install/install-nuget! name fields row dir)
  (run-inline-output! "dotnet"
                      (list "tool" "install" (lsp-install/ref fields 'package)
                            "--tool-path" (path-join dir "bin")
                            "--version" (lsp-install/ref fields 'version)))
  (lsp-install/managed-bin! "dotnet" name dir "bin" (lsp-install/ref fields 'bin) ".exe"))

(define (lsp-install/install-gem! name fields row dir)
  (run-inline-output! (if lsp-install/windows? "gem.cmd" "gem")
                      (append (list "install" "--no-document" "--install-dir" dir
                                    "--bindir" (path-join dir "bin"))
                              (lsp-install/ref fields 'packages)))
  (lsp-install/managed-bin! "gem" name dir "bin" (lsp-install/ref fields 'bin) ".bat"))

;; ── Running an install ────────────────────────────────────────────────────────

(define lsp-install/package-installers
  (list (list 'npm    lsp-install/install-npm!    '())
        (list 'cargo  lsp-install/install-cargo!  '())
        (list 'golang lsp-install/install-golang! '())
        (list 'pypi   lsp-install/install-pypi!   '())
        (list 'gem    lsp-install/install-gem!    '(("GEM_HOME" . ".") ("GEM_PATH" . ".")))
        (list 'nuget  lsp-install/install-nuget!  '())))

(define (lsp-install/download-kind? source)
  (member (lsp-install/ref source 'kind) '(github generic)))

(define (lsp-install/download-row source)
  (let* ((want   (string->symbol lsp-install/target))
         (target (call! "stdlib/find" (lambda (t) (equal? (list-ref t 0) want))
                        (lsp-install/ref source 'targets)))
         (asset  (list-ref target 1)))
    (if (equal? (lsp-install/ref source 'kind) 'generic)
        (cdr target)
        (list asset
              (string-append "https://github.com/" (lsp-install/ref source 'repo)
                             "/releases/download/" (lsp-install/ref source 'version)
                             "/" asset)
              (list-ref target 2)
              (list-ref target 3)))))

(define (lsp-install/download-install! name source row dir)
  (let ((download (lsp-install/download-row source)))
    (cons (lsp-install/install-download! name (list-ref download 1) (list-ref download 0)
                                         (list-ref download 2) (list-ref download 3)
                                         (lsp-install/row-fmt row)
                                         (car (lsp-install/row-tools row)) dir)
          '())))

(define (lsp-install/package-install! name source row dir)
  (let ((installer (assoc (lsp-install/ref source 'kind) lsp-install/package-installers)))
    (cons ((cadr installer) name source row dir)
          (list-ref installer 2))))

(define (lsp-install/run-install! name source row dir)
  (if (lsp-install/download-kind? source)
      (lsp-install/download-install! name source row dir)
      (lsp-install/package-install! name source row dir)))

(define (lsp-install/install-server! name)
  (let ((blocker (lsp-install/install-blocker name)))
    (when blocker
      (error (string-append "lsp-install/install-server!: " name ": " blocker)))
    (let ((server-fields (hash-ref lsp-install/servers name))
          (source        (lsp-install/source name))
          (dir           (lsp-install/server-dir name)))
      (lsp-install/unregister-server-languages! name)
      (call! "stdlib/delete-dir!" dir)
      (let* ((installed (lsp-install/run-install! name source (lsp-install/target-row name) dir))
             (bin-rel   (car installed)))
        (lsp-install/write-receipt! name (lsp-install/ref source 'version)
                                    bin-rel (cdr installed))
        (let ((cmd (lsp-install/ref server-fields 'command)))
          (when (which cmd)
            (log! 'info (string-append "LSP: " cmd " is also on $PATH — the managed install at "
                                       (path-join dir bin-rel) " takes precedence"))))))))
