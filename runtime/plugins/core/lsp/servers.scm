;;; core:lsp/servers.scm — LSP server install pipeline: download, verify,
;;; unpack, receipt, uninstall, catalog listing. Registration (turning a
;;; receipt into a live `register-lsp-server!` call) lives in
;;; registration.scm, required below for its catalog/receipt/path helpers.
;;; See README.md "How it works".

(require "registration.scm")

;; ── Source registry ──────────────────────────────────────────────────────────

;;; Hash: name → source-entry fields, the tagged-alist tail from
;;; lsp-sources.scm: (kind . k) (version . v) plus kind-specific fields.
(define *lsp-sources* (hash))

;;; Register one lsp-sources.scm entry: `(name field...)`.
(define (lsp/declare-source! entry)
  (set! *lsp-sources* (hash-insert *lsp-sources* (car entry) (cdr entry))))

(for-each lsp/declare-source!
  (call-with-input-file
    (path-join (runtime-dir) "scheme" "lsp-sources.scm")
    read))

;;; Hash: language → server name, derived from the shared servers catalog.
;;; See README's "Catalog and sources" for the disjointness guarantee that
;;; makes this hash-insert loop safe.
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

;;; Session-scoped set of languages already evaluated by the discovery hook,
;;; so revisiting a buffer's language doesn't re-hint (or re-suppress) it.
(define *lsp-hinted-languages* (hash))

;; ── Server name validation ───────────────────────────────────────────────────

;;; Reject a server name unsafe to join as a path segment (non-empty, not
;;; "."/"..", no path separators) — `:lsp-uninstall` takes a user-typed name
;;; straight into `lsp/server-dir`; `lsp-install` never needs this since its
;;; name always comes from the seeded `*lsp-lang->server*` hash.
(define (lsp/valid-server-name? name)
  (and (string? name)
       (> (string-length name) 0)
       (not (equal? name "."))
       (not (equal? name ".."))
       (let loop ((i 0))
         (cond ((= i (string-length name)) #t)
               ((or (equal? (substring name i (+ i 1)) "/")
                    (equal? (substring name i (+ i 1)) "\\"))
                #f)
               (else (loop (+ i 1)))))))

;; ── Receipts (write side) ────────────────────────────────────────────────────
;; receipt.scm is the install commit point, written LAST by
;; `lsp/install-server!`. Read side lives in registration.scm, required above.

;;; Escape `s` as a double-quoted Scheme string literal (mirrors
;;; scripts/sync_common.py's scheme_str).
(define (lsp/scheme-quote s)
  (string-append "\"" (string-replace (string-replace s "\\" "\\\\") "\"" "\\\"") "\""))

;;; Write `name`'s receipt — the install commit point.
(define (lsp/write-receipt! name version bin)
  (call! "stdlib/write-file" (lsp/receipt-path name)
    (string-append "((name . " (lsp/scheme-quote name) ")"
                   " (version . " (lsp/scheme-quote version) ")"
                   " (bin . " (lsp/scheme-quote bin) "))")))

;;; Verify `path`'s sha256 digest matches `expected` (either the seeded
;;; data-file literal `"sha256:<hex>"` or bare hex). On mismatch, deletes
;;; `path` and raises naming both digests.
(define (lsp/verify-sha256! path expected)
  (let* ((expected-hex (string-downcase
                          (if (starts-with? expected "sha256:")
                              (substring expected 7 (string-length expected))
                              expected)))
         (actual (string-downcase (sha256-file path))))
    (unless (equal? actual expected-hex)
      (call! "stdlib/delete-file" path)
      (error (string-append "lsp/verify-sha256!: sha256 mismatch for '" path
                            "': expected " expected-hex ", got " actual)))))

;; ── Asset format + installability ─────────────────────────────────────────────

;;; 'gz, 'zip, or #f (unsupported — .tar.*, .tgz, or a bare binary) for a
;;; github asset filename. Single source for the installability check, the
;;; tool preflight, and the install-path dispatch.
(define (lsp/asset-format asset-file)
  (cond ((ends-with? asset-file ".zip") 'zip)
        ((and (ends-with? asset-file ".gz") (not (ends-with? asset-file ".tar.gz"))) 'gz)
        (else #f)))

;;; The target tuple `(hume-target asset-file sha256 bin-path)` matching the
;;; current platform, or `#f` if `name`'s github source has none.
(define (lsp/find-target targets)
  (let ((want (string->symbol (hume-target))))
    (call! "stdlib/find" (lambda (t) (equal? (list-ref t 0) want)) targets)))

;;; #f when `name` is installable on this platform, else a human-readable
;;; reason — single source for :lsp-install's error, :lsp-servers's
;;; annotation, and the discovery hint's gate.
(define (lsp/install-blocker name)
  (cond
    ((not (hume-target)) "unsupported platform")
    ((not (hash-contains? *lsp-sources* name)) "no install source")
    (else
     (let* ((fields (hash-ref *lsp-sources* name))
            (kind   (cdr (lsp/field fields 'kind))))
       (cond
         ((equal? kind 'npm)
          (if (which "npm") #f "requires 'npm' on $PATH, which was not found"))
         ((equal? kind 'cargo)
          (if (which "cargo") #f "requires 'cargo' on $PATH, which was not found"))
         ((not (equal? kind 'github))
          (string-append "not installable (kind " (symbol->string kind) ") in v1"))
         (else
          (let ((target (lsp/find-target (cdr (lsp/field fields 'targets)))))
            (cond
              ((not target) "no prebuilt asset for this platform")
              ((not (lsp/asset-format (list-ref target 1)))
               (string-append "unsupported asset format (" (list-ref target 1) ") in v1"))
              (else #f)))))))))

;; ── Install pipeline ──────────────────────────────────────────────────────────

;;; External tool `name`'s install needs, given its blocker is already known
;;; to be #f.
(define (lsp/required-tool name)
  (let* ((fields (hash-ref *lsp-sources* name))
         (kind   (cdr (lsp/field fields 'kind))))
    (cond
      ((equal? kind 'npm) "npm")
      ((equal? kind 'cargo) "cargo")
      (else
       (let* ((target (lsp/find-target (cdr (lsp/field fields 'targets))))
              (fmt    (lsp/asset-format (list-ref target 1))))
         (cond
           ((equal? fmt 'zip) (if (equal? (hume-target) "windows-x64") "tar" "unzip"))
           (else "gzip")))))))

;;; Fail loudly, naming the tool, before any download starts.
(define (lsp/preflight! name)
  (let* ((fields (hash-ref *lsp-sources* name))
         (kind   (cdr (lsp/field fields 'kind)))
         (tool   (lsp/required-tool name)))
    (unless (which tool)
      (error (string-append "lsp/install-server!: " name " requires '" tool
                            "' on $PATH, which was not found")))
    (when (and (equal? kind 'github) (not (which "curl")))
      (error (string-append "lsp/install-server!: " name
                            " requires 'curl' on $PATH, which was not found")))))

;;; Download, verify, and unpack a github-kind release asset. Returns the
;;; bin path relative to `dir`.
(define (lsp/install-github! name fields dir)
  (let* ((repo    (cdr (lsp/field fields 'repo)))
         (version (cdr (lsp/field fields 'version)))
         (target  (lsp/find-target (cdr (lsp/field fields 'targets))))
         (asset   (list-ref target 1))
         (sha     (list-ref target 2))
         (bin     (list-ref target 3))
         (fmt     (lsp/asset-format asset))
         (archive (path-join dir asset))
         (url     (string-append "https://github.com/" repo "/releases/download/"
                                 version "/" asset)))
    ;; `dir` was only ever purged by `stdlib/delete-dir` above, never
    ;; recreated — `curl` needs the parent directory to already exist.
    (create-directory! dir)
    (run-inline-output! "curl" (list "-fsSL" "-o" archive "--" url))
    (lsp/verify-sha256! archive sha)
    (cond
      ((equal? fmt 'gz) (unpack-gz archive (path-join dir bin)))
      ((equal? fmt 'zip) (unpack-zip archive dir bin)))
    (call! "stdlib/delete-file" archive)
    (unless (path-exists? (path-join dir bin))
      (error (string-append "lsp/install-github!: " name
                            ": expected binary not found after unpack: " bin)))
    bin))

;;; Run `npm install` for an npm-kind package. Returns the bin path relative
;;; to `dir` (a `.cmd` shim on Windows — HUME's LSP transport wraps
;;; `.cmd`/`.bat` commands in `cmd /C`, cfg-gated).
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

;;; Run `cargo install` for a cargo-kind crate, rooted in the server dir.
;;; Returns the bin path relative to `dir`. `--locked` — the closest cargo
;;; analog to the sha256 pin github assets get — builds with upstream's
;;; published Cargo.lock.
(define (lsp/install-cargo! name fields dir)
  (let* ((crate    (cdr (lsp/field fields 'crate)))
         (version  (cdr (lsp/field fields 'version)))
         (bin      (cdr (lsp/field fields 'bin)))
         (windows? (equal? (hume-target) "windows-x64"))
         (bin-rel  (string-append "bin/" bin (if windows? ".exe" ""))))
    (run-inline-output! "cargo"
                        (list "install" "--locked" "--root" dir "--"
                              (string-append crate "@" version)))
    (unless (path-exists? (path-join dir bin-rel))
      (error (string-append "lsp/install-cargo!: " name
                            ": expected binary not found after cargo install: " bin-rel)))
    bin-rel))

;;; Install (or reinstall) `name` from its declared source, always from a
;;; clean slate — see README's "Server install and registration" for the
;;; pipeline steps. Registration is the caller's job, `lsp/lsp-install-or-report!`.
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
    (call! "stdlib/delete-dir" dir)
    (let ((bin-rel (cond
                     ((equal? kind 'github) (lsp/install-github! name source-fields dir))
                     ((equal? kind 'cargo)  (lsp/install-cargo! name source-fields dir))
                     (else                  (lsp/install-npm! name source-fields dir)))))
      (lsp/write-receipt! name (cdr (lsp/field source-fields 'version)) bin-rel)
      (let ((cmd (cdr (lsp/field server-fields 'command))))
        (when (which cmd)
          (log! 'info (string-append "LSP: " cmd " is also on $PATH — the managed install at "
                                     (path-join dir bin-rel) " takes precedence")))))))

;; ── Commands ──────────────────────────────────────────────────────────────────

;;; Runs `thunk` under the cross-process install lock. See README's
;;; "Install lock" for the lock path, the re-raise hazard this avoids, and
;;; the return-value contract.
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

;;; Install `name` if not already at the seeded version, reporting a
;;; guided-retry hint on failure when a prior install dir existed. Runs
;;; *outside* `lsp/with-install-lock!`, after that combinator already
;;; released the lock — see README's "Install lock".
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

(define-typed-command! "lsp-install"
  "Download and verify the language server for a language (default: the current buffer's language), then register it."
  (lambda (arg)
    (let ((lang (call! "stdlib/resolve-lang-arg" "lsp-install" arg)))
      (cond
        ((not lang) (begin))
        ((not (hash-contains? *lsp-lang->server* lang))
         (log! 'info (string-append "lsp-install: no language server is seeded for \"" lang "\"")))
        (else
         (lsp/lsp-install-or-report! (hash-ref *lsp-lang->server* lang))))))
  #:inline-output #t)

(define-typed-command! "lsp-uninstall"
  "Shut down and remove an installed language server by name."
  (lambda (arg)
    (cond
      ((not (string? arg))
       (log! 'info "lsp-uninstall: requires a server name, e.g. :lsp-uninstall rust-analyzer"))
      ;; Stays 'warn, not 'info: this also rejects a path-traversal name
      ;; (e.g. "../plugins") — a security-relevant refusal worth a
      ;; persistent :messages record, not an ordinary usage typo. Same
      ;; reasoning as plum/fetch-raw-query's grammar-name rejection.
      ((not (lsp/valid-server-name? arg))
       (log! 'warn (string-append "lsp-uninstall: invalid server name: " arg)))
      (else
        (let* ((name arg)
               (dir  (lsp/server-dir name)))
          ;; Idempotent no-op when unseeded/never-registered — matches
          ;; unregister-lsp-server!'s own idempotency. Orphan (dir exists,
          ;; no seeded entry): skip this and only remove the directory below.
          (when (hash-contains? (lsp/servers-catalog) name)
            (for-each (lambda (lang-entry) (unregister-lsp-server! (car lang-entry)))
                      (cdr (lsp/field (hash-ref (lsp/servers-catalog) name) 'languages))))
          ;; The delete itself is cross-process-lock-guarded, same as
          ;; install. Deferred to `after 0` so the unregister above has
          ;; already shut down any running client; the lock is acquired
          ;; there, right before the delete, not any earlier.
          (if (path-exists? dir)
              (begin
                (log! 'info (string-append "LSP: shutting down and removing " name "..."))
                (after 0 (lambda ()
                           (when (lsp/with-install-lock! (string-append "uninstall " name)
                                   (lambda () (call! "stdlib/delete-dir" dir)))
                             (log! 'info (string-append "LSP: removed " name))))))
              (log! 'info (string-append "LSP: nothing to uninstall for " name))))))))

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

;;; Once per language per session — see README's "Discovery hint".
(register-hook! 'on-language-set
  (lambda (bid lang)
    (when (and (string? lang) (not (hash-contains? *lsp-hinted-languages* lang)))
      (set! *lsp-hinted-languages* (hash-insert *lsp-hinted-languages* lang #t))
      (when (hash-contains? *lsp-lang->server* lang)
        (let* ((name    (hash-ref *lsp-lang->server* lang))
               (blocker (lsp/install-blocker name)))
          (when (and (not blocker)
                     (not (lsp-registered-for-language? lang))
                     (not (lsp/read-receipt name)))
            (log! 'warn (string-append "LSP: language server '" name
                                       "' is available for " lang " — run :lsp-install"))))))))
