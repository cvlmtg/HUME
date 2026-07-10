;;; core:plum/servers.scm

(require "lib.scm")
(require-builtin steel/vectors)
(provide plum/declare-lsp-server! plum/declare-lsp-source! plum/register-installed-servers!)

;; ── Server registration + source registries ──────────────────────────────────

;;; Hash: name → server-entry fields, the tagged-alist tail from
;;; lsp-servers.scm: (languages ...) (command . cmd) (args ...) (settings ...).
(define *plum-lsp-servers* (hash))

;;; Hash: name → source-entry fields, the tagged-alist tail from
;;; lsp-sources.scm: (kind . k) (version . v) plus kind-specific fields.
(define *plum-lsp-sources* (hash))

;;; Hash: language → server name. Built while declaring servers; languages
;;; are disjoint across servers by sync-time guarantee (see docs/LSP-INSTALL.md
;;; "v1 scope and limitations").
(define *plum-lang->server* (hash))

;;; Session-scoped set of languages already evaluated by the discovery hook,
;;; so revisiting a buffer's language doesn't re-hint (or re-suppress) it.
(define *plum-hinted-languages* (hash))

;;; Register one lsp-servers.scm entry: `(name field...)`.
(define (plum/declare-lsp-server! entry)
  (let* ((name  (car entry))
         (fields (cdr entry))
         (langs (cdr (plum/field fields 'languages))))
    (set! *plum-lsp-servers* (hash-insert *plum-lsp-servers* name fields))
    (for-each
      (lambda (lang-entry)
        (set! *plum-lang->server* (hash-insert *plum-lang->server* (car lang-entry) name)))
      langs)))

;;; Register one lsp-sources.scm entry: `(name field...)`.
(define (plum/declare-lsp-source! entry)
  (set! *plum-lsp-sources* (hash-insert *plum-lsp-sources* (car entry) (cdr entry))))

;; ── Field access ──────────────────────────────────────────────────────────────
;;
;; Entries in both catalogs are tagged alists — `(key . value)` (a scalar or
;; #(...) vector leaf) or `(key sub…)` (a nested list) — never positional
;; tuples, since install/registration records are heterogeneous. `car` works
;; on both shapes, which is what makes a single lookup helper possible.

;;; First element of `fields` whose car is `key` (a symbol), or `#f`.
(define (plum/field fields key)
  (cond ((null? fields) #f)
        ((equal? (car (car fields)) key) (car fields))
        (else (plum/field (cdr fields) key))))

;; ── Paths ─────────────────────────────────────────────────────────────────────

(define (plum/servers-dir) (path-join (data-dir) "servers"))
(define (plum/server-dir name) (path-join (plum/servers-dir) name))
(define (plum/receipt-path name) (path-join (plum/server-dir name) "receipt.scm"))

;; ── Receipts ──────────────────────────────────────────────────────────────────
;;
;; receipt.scm is the install commit point: pure data
;; `((name . "X") (version . "V") (bin . "relative/bin/path"))`, written LAST
;; by `plum/install-server!`. A server dir without a readable receipt is an
;; interrupted install (see docs/LSP-INSTALL.md "Installation layout").

;;; Read `name`'s receipt, or `#f` if missing/unreadable (interrupted install).
(define (plum/read-receipt name)
  (with-handler (lambda (err) #f)
    (call-with-input-file (plum/receipt-path name) read)))

(define (plum/receipt-version receipt) (cdr (plum/field receipt 'version)))
(define (plum/receipt-bin receipt) (cdr (plum/field receipt 'bin)))

;;; Escape `s` as a double-quoted Scheme string literal (mirrors
;;; scripts/sync_common.py's scheme_str — receipts are the one place PLUM
;;; writes Scheme data instead of just reading it).
(define (plum/scheme-quote s)
  (string-append "\"" (string-replace (string-replace s "\\" "\\\\") "\"" "\\\"") "\""))

;;; Write `name`'s receipt — the install commit point.
(define (plum/write-receipt! name version bin)
  (write-file (plum/receipt-path name)
    (string-append "((name . " (plum/scheme-quote name) ")"
                   " (version . " (plum/scheme-quote version) ")"
                   " (bin . " (plum/scheme-quote bin) "))")))

;; ── Settings conversion ───────────────────────────────────────────────────────
;;
;; Seeded settings are nested alists whose entries take one of three shapes
;; (see docs/LSP-INSTALL.md "Seeded data format"): `(key . scalar)`,
;; `(key . #(elem…))` for a JSON array, or `(key entry…)` for a nested
;; object. `steel_to_json` has no case for a raw vector, so every `#(...)`
;; must become a Steel list before it can reach `#:settings`.

;;; Convert a #(...) vector into a Steel list.
(define (plum/vector->steel-list v)
  (let loop ((i (- (vector-length v) 1)) (acc '()))
    (if (< i 0) acc (loop (- i 1) (cons (vector-ref v i) acc)))))

;;; Convert a settings entry list into a Steel hash suitable for `#:settings`.
;;; Every entry must be a pair or a (nonempty-or-not) list — the seeded data
;;; is canonicalised at sync time, so anything else is a sync bug, not
;;; something to skip over.
(define (plum/settings->hash entries)
  (let loop ((entries entries) (h (hash)))
    (cond
      ((null? entries) h)
      ((not (pair? (car entries)))
       (error (string-append "plum/settings->hash: malformed settings entry: "
                             (to-string (car entries)))))
      (else
       (let* ((entry (car entries))
              (key   (car entry))
              (value (if (list? entry)
                         (plum/settings->hash (cdr entry))
                         (let ((v (cdr entry)))
                           (if (vector? v) (plum/vector->steel-list v) v)))))
         (loop (cdr entries) (hash-insert h key value)))))))

;; ── Asset format + installability ─────────────────────────────────────────────

;;; 'gz, 'zip, or #f (unsupported — .tar.*, .tgz, or a bare binary) for a
;;; github asset filename. Single source for the installability check, the
;;; tool preflight, and the install-path dispatch.
(define (plum/asset-format asset-file)
  (cond ((ends-with? asset-file ".zip") 'zip)
        ((and (ends-with? asset-file ".gz") (not (ends-with? asset-file ".tar.gz"))) 'gz)
        (else #f)))

;;; The target tuple `(hume-target asset-file sha256 bin-path)` matching the
;;; current platform, or `#f` if `name`'s github source has none.
(define (plum/find-target targets)
  (let ((want (string->symbol (hume-target))))
    (plum/find (lambda (t) (equal? (list-ref t 0) want)) targets)))

;;; #f when `name` is installable on this platform, else a human-readable
;;; reason — single source for :lsp-install's error, :lsp-servers's
;;; annotation, and the discovery hint's gate.
(define (plum/install-blocker name)
  (cond
    ((not (hume-target)) "unsupported platform")
    ((not (hash-contains? *plum-lsp-sources* name)) "no install source")
    (else
     (let* ((fields (hash-ref *plum-lsp-sources* name))
            (kind   (cdr (plum/field fields 'kind))))
       (cond
         ((equal? kind 'npm) #f)
         ((not (equal? kind 'github))
          (string-append "not installable (kind " (symbol->string kind) ") in v1"))
         (else
          (let ((target (plum/find-target (cdr (plum/field fields 'targets)))))
            (cond
              ((not target) "no prebuilt asset for this platform")
              ((not (plum/asset-format (list-ref target 1)))
               (string-append "unsupported asset format (" (list-ref target 1) ") in v1"))
              (else #f)))))))))

;; ── Registration helper (shared by the scan and by install) ──────────────────

;;; Register `name` for every language it serves, with `cmd` as the server
;;; command. `args`/`settings` are shared across a multi-language server;
;;; only root markers vary per language (see docs/LSP-INSTALL.md "Seeded
;;; data format").
(define (plum/register-server-languages! name cmd)
  (let* ((fields   (hash-ref *plum-lsp-servers* name))
         (langs    (cdr (plum/field fields 'languages)))
         (args     (cdr (plum/field fields 'args)))
         (settings-entries (cdr (plum/field fields 'settings)))
         (settings (if (null? settings-entries) #f (plum/settings->hash settings-entries))))
    (for-each
      (lambda (lang-entry)
        (register-lsp-server! (car lang-entry)
                               #:command cmd
                               #:args args
                               #:root-markers (cdr lang-entry)
                               #:settings settings))
      langs)))

;; ── Startup server registration ───────────────────────────────────────────────

;;; Passive: registers already-installed servers only (a readable receipt
;;; naming a seeded server), no subprocess, no network. See README.md §
;;; startup registration.
(define (plum/register-installed-servers!)
  (let ((sdir (plum/servers-dir)))
    (when (path-exists? sdir)
      (for-each
        (lambda (name)
          (let ((receipt (plum/read-receipt name)))
            (cond
              ((not receipt)
               (log! 'warn (string-append "PLUM: interrupted install of " name
                                          " — run :lsp-install to redo, or delete the directory")))
              ((not (hash-contains? *plum-lsp-servers* name))
               (log! 'warn (string-append "PLUM: orphan server " name
                                          " — not in the seeded catalog, run :lsp-uninstall to remove")))
              (else
               (plum/register-server-languages!
                 name
                 (path-join (plum/server-dir name) (plum/receipt-bin receipt)))))))
        (filter plum/valid-dir-entry? (list-dir sdir))))))

;; ── Install pipeline ──────────────────────────────────────────────────────────

;;; External tool `name`'s install needs, given its blocker is already known
;;; to be #f. See docs/LSP-INSTALL.md "Required external tools".
(define (plum/required-tool name)
  (let* ((fields (hash-ref *plum-lsp-sources* name))
         (kind   (cdr (plum/field fields 'kind))))
    (if (equal? kind 'npm)
        "npm"
        (let* ((target (plum/find-target (cdr (plum/field fields 'targets))))
               (fmt    (plum/asset-format (list-ref target 1))))
          (cond
            ((equal? fmt 'zip) (if (equal? (hume-target) "windows-x64") "tar" "unzip"))
            (else "gzip"))))))

;;; Fail loudly, naming the tool, before any download starts.
(define (plum/preflight! name)
  (let* ((fields (hash-ref *plum-lsp-sources* name))
         (kind   (cdr (plum/field fields 'kind)))
         (tool   (plum/required-tool name)))
    (unless (exe-on-path? tool)
      (error (string-append "plum/install-server!: " name " requires '" tool
                            "' on $PATH, which was not found")))
    (when (and (equal? kind 'github) (not (exe-on-path? "curl")))
      (error (string-append "plum/install-server!: " name
                            " requires 'curl' on $PATH, which was not found")))))

;;; Download, verify, and unpack a github-kind release asset. Returns the
;;; bin path relative to `dir`.
(define (plum/install-github! name fields dir)
  (let* ((repo    (cdr (plum/field fields 'repo)))
         (version (cdr (plum/field fields 'version)))
         (target  (plum/find-target (cdr (plum/field fields 'targets))))
         (asset   (list-ref target 1))
         (sha     (list-ref target 2))
         (bin     (list-ref target 3))
         (fmt     (plum/asset-format asset))
         (archive (path-join dir asset))
         (url     (string-append "https://github.com/" repo "/releases/download/"
                                 version "/" asset)))
    (curl-fetch url archive)
    (verify-sha256! archive sha)
    (cond
      ((equal? fmt 'gz) (unpack-gz archive (path-join dir bin)))
      ((equal? fmt 'zip) (unpack-zip archive dir bin)))
    (delete-file archive)
    (unless (path-exists? (path-join dir bin))
      (error (string-append "plum/install-github!: " name
                            ": expected binary not found after unpack: " bin)))
    bin))

;;; Run `npm install` for an npm-kind package. Returns the bin path relative
;;; to `dir` (a `.cmd` shim on Windows — HUME's LSP transport wraps
;;; `.cmd`/`.bat` commands in `cmd /C`, cfg-gated).
(define (plum/install-npm! name fields dir)
  (let* ((packages (cdr (plum/field fields 'packages)))
         (bin      (cdr (plum/field fields 'bin)))
         (windows? (equal? (hume-target) "windows-x64"))
         (bin-rel  (string-append "node_modules/.bin/" bin (if windows? ".cmd" ""))))
    (npm-install! dir packages)
    (unless (path-exists? (path-join dir bin-rel))
      (error (string-append "plum/install-npm!: " name
                            ": expected binary not found after npm install: " bin-rel)))
    bin-rel))

;;; Install (or reinstall) a single server from its declared source, always
;;; from a clean slate — this doubles as the repair/upgrade path. See
;;; docs/LSP-INSTALL.md "Commands and lifecycle" for the
;;; reinstall-over-a-running-client behaviour:
;;;   1. blocker check + tool preflight
;;;   2. unregister every seeded language (idempotent; queued, applies at
;;;      end-of-eval, after which any running client is reaped)
;;;   3. delete-dir — purge any existing install; the receipt dies with it,
;;;      so an interruption from here on is self-describing
;;;   4. download + verify + unpack (github), or npm-install! (npm)
;;;   5. write receipt — the commit point
;;;   6. register every seeded language with the fresh absolute bin path
;;;   7. $PATH notice, if the seeded command also resolves there
(define (plum/install-server! name)
  (let ((blocker (plum/install-blocker name)))
    (when blocker
      (error (string-append "plum/install-server!: " name ": " blocker))))
  (plum/preflight! name)
  (let* ((server-fields (hash-ref *plum-lsp-servers* name))
         (source-fields (hash-ref *plum-lsp-sources* name))
         (kind          (cdr (plum/field source-fields 'kind)))
         (dir           (plum/server-dir name)))
    (for-each (lambda (lang-entry) (unregister-lsp-server! (car lang-entry)))
              (cdr (plum/field server-fields 'languages)))
    (delete-dir dir)
    (let ((bin-rel (if (equal? kind 'github)
                        (plum/install-github! name source-fields dir)
                        (plum/install-npm! name source-fields dir))))
      (plum/write-receipt! name (cdr (plum/field source-fields 'version)) bin-rel)
      (plum/register-server-languages! name (path-join dir bin-rel))
      (let ((cmd (cdr (plum/field server-fields 'command))))
        (when (exe-on-path? cmd)
          (log! 'info (string-append "PLUM: " cmd " is also on $PATH — the managed install at "
                                     (path-join dir bin-rel) " takes precedence")))))))

;; ── Commands ──────────────────────────────────────────────────────────────────

;;; Resolve the target language for `:lsp-install`: a string argument wins;
;;; otherwise the current buffer's language. `arg` is a string only when the
;;; user typed one — the minibuffer passes the default count 1 otherwise.
(define (plum/resolve-lsp-lang-arg arg)
  (if (string? arg) arg (buffer-language (current-buffer))))

;;; Install `name` if not already at the seeded version, reporting a
;;; guided-retry hint on failure when a prior install dir existed (covers
;;; the Windows locked-file case; a one-shot retry on Unix).
(define (plum/lsp-install-or-report! name)
  (let* ((receipt (plum/read-receipt name))
         (source  (if (hash-contains? *plum-lsp-sources* name)
                      (hash-ref *plum-lsp-sources* name)
                      #f)))
    (if (and receipt source
             (equal? (plum/receipt-version receipt) (cdr (plum/field source 'version))))
        (log! 'info (string-append "PLUM: " name " already installed (v"
                                   (plum/receipt-version receipt) ") — up to date"))
        (let ((had-dir? (path-exists? (plum/server-dir name))))
          (log! 'info (string-append "PLUM: installing " name "..."))
          (with-handler
            (lambda (err)
              (log! 'error (string-append "PLUM: install failed: " (to-string err)))
              (when had-dir?
                (log! 'info "PLUM: if the server was running it has now been shut down — run :lsp-install again")))
            (plum/install-server! name))))))

(define-command! "lsp-install"
  "Download, verify, and register the language server for a language (default: the current buffer's language)."
  (lambda (arg)
    (let ((lang (plum/resolve-lsp-lang-arg arg)))
      (cond
        ((not (string? lang))
         (log! 'warn "lsp-install: no language given and current buffer has no language set"))
        ((not (hash-contains? *plum-lang->server* lang))
         (log! 'warn (string-append "lsp-install: no language server is seeded for \"" lang "\"")))
        (else
         (plum/lsp-install-or-report! (hash-ref *plum-lang->server* lang))))))
  #:inline-output #t)

(define-command! "lsp-uninstall"
  "Shut down and remove an installed language server by name."
  (lambda (arg)
    (if (not (string? arg))
        (log! 'warn "lsp-uninstall: requires a server name, e.g. :lsp-uninstall rust-analyzer")
        (let* ((name arg)
               (dir  (plum/server-dir name)))
          ;; Idempotent no-op when unseeded/never-registered — matches
          ;; unregister-lsp-server!'s own idempotency. Orphan (dir exists,
          ;; no seeded entry): skip this and only remove the directory below.
          (when (hash-contains? *plum-lsp-servers* name)
            (for-each (lambda (lang-entry) (unregister-lsp-server! (car lang-entry)))
                      (cdr (plum/field (hash-ref *plum-lsp-servers* name) 'languages))))
          (if (path-exists? dir)
              (begin
                (log! 'info (string-append "PLUM: shutting down and removing " name "..."))
                (after 0 (lambda ()
                           (delete-dir dir)
                           (log! 'info (string-append "PLUM: removed " name)))))
              (log! 'info (string-append "PLUM: nothing to uninstall for " name)))))))

(define-command! "lsp-servers"
  "Log the LSP server catalog: languages, seeded version, and install status."
  (lambda ()
    (for-each
      (lambda (name)
        (let* ((receipt (plum/read-receipt name))
               (source  (if (hash-contains? *plum-lsp-sources* name)
                            (hash-ref *plum-lsp-sources* name)
                            #f))
               (langs   (map car (cdr (plum/field (hash-ref *plum-lsp-servers* name) 'languages))))
               (status
                 (cond
                   (receipt
                    (let ((installed (plum/receipt-version receipt))
                          (seeded    (if source (cdr (plum/field source 'version)) #f)))
                      (if (and seeded (not (equal? installed seeded)))
                          (string-append "installed v" installed " — update available (v" seeded ")")
                          (string-append "installed v" installed))))
                   (else
                    (let ((blocker (plum/install-blocker name)))
                      (if blocker blocker "not installed"))))))
          (displayln (string-append name " [" (string-join langs ", ") "]: " status))))
      (hash-keys->list *plum-lsp-servers*))
    (log! 'info (string-append "PLUM: " (number->string (length (hash-keys->list *plum-lsp-servers*)))
                               " seeded servers")))
  #:inline-output #t)

;; ── Discovery hint ────────────────────────────────────────────────────────────
;;
;; Once per language per session: if a buffer's language has an installable
;; seeded server that isn't registered yet, suggest :lsp-install. Never
;; hints a suggestion that would fail — the dedup marker is set on the
;; language's first evaluation regardless of outcome, so a language that
;; doesn't qualify (no seeded server, blocked, already registered) is never
;; re-evaluated this session either. `'warn`, not `'info`: an `'info`
;; message only flashes on the status line and is never written to
;; `:messages` (HUME's `Severity::Info` is display-only, not logged) — a
;; discoverability nudge the user can miss at the moment it fires must stay
;; reviewable afterward.
(register-hook! 'on-language-set
  (lambda (bid lang)
    (when (and (string? lang) (not (hash-contains? *plum-hinted-languages* lang)))
      (set! *plum-hinted-languages* (hash-insert *plum-hinted-languages* lang #t))
      (when (hash-contains? *plum-lang->server* lang)
        (let* ((name    (hash-ref *plum-lang->server* lang))
               (blocker (plum/install-blocker name)))
          (when (and (not blocker) (not (lsp-registered-for-language? lang)))
            (log! 'warn (string-append "PLUM: language server '" name
                                       "' is available for " lang " — run :lsp-install"))))))))
