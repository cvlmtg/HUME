;;; core:lsp/servers.scm — LSP server install pipeline: download, verify,
;;; unpack, receipt, uninstall, catalog listing. Registration (turning a
;;; receipt into a live `register-lsp-server!` call) lives in
;;; registration.scm, required below for its catalog/receipt/path helpers —
;;; both files are the same plugin, so a direct call replaces what used to
;;; be a cross-plugin `call!` notify to core:lsp from core:plum.

(require "registration.scm")

;; ── Filesystem + list-search helpers ────────────────────────────────────────
;;
;; Thin wrappers over Steel's `steel/filesystem`/`steel/ports`, matching the
;; contracts of the same-named helpers in plum/lib.scm (used by PLUM's own
;; plugin/grammar install pipelines) — duplicated here rather than shared,
;; since plugins never require each other's modules (see docs/ROADMAP.md
;; "Plugin namespace isolation").

;;; First element of `lst` satisfying `pred?`, or `#f`.
(define (lsp/find pred? lst)
  (cond ((null? lst) #f)
        ((pred? (car lst)) (car lst))
        (else (lsp/find pred? (cdr lst)))))

;;; Write `content` to `path`, creating or truncating it.
(define (lsp/write-file path content)
  (let ([port (open-output-file path)])
    (write-string content port)
    (close-output-port port)))

;;; Recursively delete `dir`. Idempotent — a missing directory is not an
;;; error; several call sites (e.g. clearing a stale install before a
;;; reinstall) rely on being able to call this whether or not anything is
;;; there yet.
(define (lsp/delete-dir dir)
  (when (path-exists? dir)
    (delete-directory! dir)))

;;; Delete the file at `path`. Idempotent — a missing file is not an error;
;;; cleanup-on-failure call sites (e.g. removing a partial download) must
;;; tolerate the file never having been created.
(define (lsp/delete-file path)
  (when (path-exists? path)
    (delete-file! path)))

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

;;; Hash: language → server name, derived from the shared servers catalog
;;; (registration.scm's `lsp/servers-catalog`) — languages are disjoint
;;; across servers by sync-time guarantee (see docs/LSP-INSTALL.md "v1 scope
;;; and limitations").
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

;;; Reject a server name unsafe to join as a single path segment — must be
;;; non-empty, not "." or "..", and free of path separators. `:lsp-uninstall`
;;; takes a user-typed name straight into `lsp/server-dir`; `lsp-install`
;;; never needs this since its name always comes from the seeded
;;; `*lsp-lang->server*` hash, never the raw argument.
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
;;
;; receipt.scm is the install commit point: pure data
;; `((name . "X") (version . "V") (bin . "relative/bin/path"))`, written LAST
;; by `lsp/install-server!`. A server dir without a readable receipt is an
;; interrupted install (see docs/LSP-INSTALL.md "Installation layout"). Read
;; side (`lsp/read-receipt`, `lsp/receipt-version`, `lsp/receipt-bin`,
;; path helpers) lives in registration.scm, required above.

;;; Escape `s` as a double-quoted Scheme string literal (mirrors
;;; scripts/sync_common.py's scheme_str — receipts are the one place this
;;; plugin writes Scheme data instead of just reading it).
(define (lsp/scheme-quote s)
  (string-append "\"" (string-replace (string-replace s "\\" "\\\\") "\"" "\\\"") "\""))

;;; Write `name`'s receipt — the install commit point.
(define (lsp/write-receipt! name version bin)
  (lsp/write-file (lsp/receipt-path name)
    (string-append "((name . " (lsp/scheme-quote name) ")"
                   " (version . " (lsp/scheme-quote version) ")"
                   " (bin . " (lsp/scheme-quote bin) "))")))

;;; Verify `path`'s sha256 digest matches `expected` (either the seeded
;;; data-file literal `"sha256:<hex>"` or bare hex; ASCII-case-insensitive).
;;; On mismatch, deletes `path` and raises naming both digests. Hashing is
;;; the sandbox-free `sha256-file` builtin; the compare and delete-on-mismatch
;;; happen here.
(define (lsp/verify-sha256! path expected)
  (let* ((expected-hex (string-downcase
                          (if (starts-with? expected "sha256:")
                              (substring expected 7 (string-length expected))
                              expected)))
         (actual (string-downcase (sha256-file path))))
    (unless (equal? actual expected-hex)
      (lsp/delete-file path)
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
    (lsp/find (lambda (t) (equal? (list-ref t 0) want)) targets)))

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
;;; to be #f. See docs/LSP-INSTALL.md "Required external tools".
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
    ;; `dir` was only ever purged by `lsp/delete-dir` above, never recreated —
    ;; `curl` needs the parent directory to already exist.
    (create-directory! dir)
    (run-inline-output! "curl" (list "-fsSL" "-o" archive "--" url))
    (lsp/verify-sha256! archive sha)
    (cond
      ((equal? fmt 'gz) (unpack-gz archive (path-join dir bin)))
      ((equal? fmt 'zip) (unpack-zip archive dir bin)))
    (lsp/delete-file archive)
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
;;; Returns the bin path relative to `dir` (cargo's own --root layout:
;;; `bin/<name>`, `.exe` on Windows). `--locked` builds with upstream's
;;; published Cargo.lock — the closest cargo analog to the sha256 pin
;;; github assets get. cargo creates `dir` itself.
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

;;; Install (or reinstall) a single server from its declared source, always
;;; from a clean slate — this doubles as the repair/upgrade path. See
;;; docs/LSP-INSTALL.md "Commands and lifecycle" for the
;;; reinstall-over-a-running-client behaviour:
;;;   1. blocker check + tool preflight
;;;   2. unregister every seeded language (idempotent; queued, applies at
;;;      end-of-eval, after which any running client is reaped)
;;;   3. lsp/delete-dir — purge any existing install; the receipt dies with
;;;      it, so an interruption from here on is self-describing
;;;   4. download + verify + unpack (github), npm install (npm), or cargo
;;;      install (cargo)
;;;   5. write receipt — the commit point
;;;   6. $PATH notice, if the seeded command also resolves there
;;; Registration is not this function's job — the caller re-scans afterward
;;; (see `lsp/lsp-install-or-report!`).
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
    (lsp/delete-dir dir)
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

;;; Resolve the target language for `:lsp-install`: a string argument wins;
;;; otherwise the current buffer's language. `arg` is a string only when the
;;; user typed one — the minibuffer passes the default count 1 otherwise.
(define (lsp/resolve-lsp-lang-arg arg)
  (if (string? arg) arg (buffer-language (current-buffer))))

;;; Runs `thunk` (typically an install/uninstall body) under the
;;; cross-process install lock (`<data>/servers/.install-lock`) — a second
;;; HUME process installing/uninstalling at the same time is refused, not
;;; interleaved — releasing it exactly once regardless of outcome. `what`
;;; names the operation for the failure log line. Never re-raises `thunk`'s
;;; error through an outer with-handler: a nested with-handler re-raising a
;;; native-builtin error corrupts the Steel VM's continuation stack (see
;;; LESSONS.md) — every failure path here terminates in a plain `log!`, not
;;; a re-raise. Returns #t on success, #f on any failure — a lock the
;;; caller couldn't acquire (another process holds it) and a `thunk` that
;;; raised both collapse to the same #f; distinguishing them would need
;;; richer plumbing this plugin has no caller for.
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
;;; guided-retry hint on failure when a prior install dir existed (covers
;;; the Windows locked-file case; a one-shot retry on Unix).
;;;
;;; The post-install rescan (`lsp/register-installed-servers!`) sees
;;; `lsp/install-server!`'s queued `unregister-lsp-server!` for every one of
;;; `name`'s languages through `lsp-registered-for-language?`'s same-eval
;;; read-through, even though that op hasn't applied yet (queued ops apply
;;; at end-of-eval) — so the rescan's no-clobber filter correctly re-admits
;;; those languages instead of skipping them. Run *outside*
;;; `lsp/with-install-lock!`: it runs only after that combinator has already
;;; released the lock, so a failure here is a distinct, uncaught error
;;; (reported by the command dispatcher, see `Editor::run_steel_command` in
;;; hume-editor/src/editor/dispatch.rs) rather than being mislabeled
;;; "install failed" or triggering a second, ownership-blind
;;; `release-install-lock!` call.
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

(define-command! "lsp-install"
  "Download and verify the language server for a language (default: the current buffer's language), then register it."
  (lambda (arg)
    (let ((lang (lsp/resolve-lsp-lang-arg arg)))
      (cond
        ((not (string? lang))
         (log! 'warn "lsp-install: no language given and current buffer has no language set"))
        ((not (hash-contains? *lsp-lang->server* lang))
         (log! 'warn (string-append "lsp-install: no language server is seeded for \"" lang "\"")))
        (else
         (lsp/lsp-install-or-report! (hash-ref *lsp-lang->server* lang))))))
  #:inline-output #t)

(define-command! "lsp-uninstall"
  "Shut down and remove an installed language server by name."
  (lambda (arg)
    (cond
      ((not (string? arg))
       (log! 'warn "lsp-uninstall: requires a server name, e.g. :lsp-uninstall rust-analyzer"))
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
          ;; install — a second HUME process must never race this one's
          ;; delete-dir. Deferred to `after 0` so the unregister above has
          ;; already shut down any running client; the lock is acquired
          ;; there, right before the delete, not any earlier.
          (if (path-exists? dir)
              (begin
                (log! 'info (string-append "LSP: shutting down and removing " name "..."))
                (after 0 (lambda ()
                           (when (lsp/with-install-lock! (string-append "uninstall " name)
                                   (lambda () (lsp/delete-dir dir)))
                             (log! 'info (string-append "LSP: removed " name))))))
              (log! 'info (string-append "LSP: nothing to uninstall for " name))))))))

(define-command! "lsp-servers"
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
;;
;; Once per language per session: if a buffer's language has a seeded server
;; that isn't installed yet, suggest :lsp-install. Never hints a suggestion
;; that would fail — the dedup marker is set on the language's first
;; evaluation regardless of outcome, so a language that doesn't qualify (no
;; seeded server, blocked, already registered, already installed) is never
;; re-evaluated this session either. `'warn`, not `'info`: an `'info`
;; message only flashes on the status line and is never written to
;; `:messages` (HUME's `Severity::Info` is display-only, not logged) — a
;; discoverability nudge the user can miss at the moment it fires must stay
;; reviewable afterward. Only fires while core:lsp is loaded/active — a
;; setup running only core:plum gets no LSP hints, matching the rest of the
;; feature (see docs/LSP-INSTALL.md "Registration model").
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
