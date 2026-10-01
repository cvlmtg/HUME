;;; core:lsp/registration.scm — the LSP server catalog, receipt/path
;;; primitives, and the scan that turns an installed server into a live
;;; registration. `servers.scm` requires this file for its read-side
;;; helpers. See docs/servers.md.

(provide lsp/register-installed-servers! lsp/field lsp/servers-dir lsp/server-dir
         lsp/receipt-path lsp/read-receipt lsp/receipt-bin lsp/receipt-version
         lsp/receipt-env-dirs
         lsp/servers-catalog)

;; ── Server catalog ────────────────────────────────────────────────────────────

(define *lsp-servers* (hash))

(define (lsp/declare-server! entry)
  (set! *lsp-servers* (hash-insert *lsp-servers* (car entry) (cdr entry))))

(for-each lsp/declare-server!
  (call-with-input-file
    (path-join (runtime-dir) "scheme" "lsp-servers.scm")
    read))

(define (lsp/servers-catalog) *lsp-servers*)

;; ── Field access ──────────────────────────────────────────────────────────────

(define (lsp/field fields key)
  (cond ((null? fields) #f)
        ((equal? (car (car fields)) key) (car fields))
        (else (lsp/field (cdr fields) key))))

;; ── Paths ─────────────────────────────────────────────────────────────────────

(define (lsp/servers-dir) (path-join (data-dir) "servers"))
(define (lsp/server-dir name) (path-join (lsp/servers-dir) name))
(define (lsp/receipt-path name) (path-join (lsp/server-dir name) "receipt.scm"))

;; ── Receipts ──────────────────────────────────────────────────────────────────

(define (lsp/read-receipt name)
  (with-handler (lambda (err) #f)
    (call-with-input-file (lsp/receipt-path name) read)))

(define (lsp/receipt-bin receipt) (cdr (lsp/field receipt 'bin)))
(define (lsp/receipt-version receipt) (cdr (lsp/field receipt 'version)))

;; ("KEY" . "subpath") pairs; each value is relative to the server dir.
(define (lsp/receipt-env-dirs receipt)
  (let ((field (lsp/field receipt 'env-dirs)))
    (if field (cdr field) '())))

;; ── Registration ──────────────────────────────────────────────────────────────

(define (lsp/register-server-languages! name cmd env)
  (let* ((fields   (hash-ref *lsp-servers* name))
         (langs    (filter (lambda (lang-entry) (not (lsp-registered-for-language? (car lang-entry))))
                            (cdr (lsp/field fields 'languages))))
         (args     (cdr (lsp/field fields 'args)))
         (config-json (cdr (lsp/field fields 'config)))
         (config (if (null? config-json) #f (json-parse config-json))))
    (for-each
      (lambda (lang-entry)
        (register-lsp-server! (car lang-entry)
                               #:command cmd
                               #:args args
                               #:root-markers (cdr lang-entry)
                               #:init-options config
                               #:settings config
                               #:env env))
      langs)))

;; ── Startup server registration ───────────────────────────────────────────────

(define (lsp/register-installed-servers!)
  (let ((sdir (lsp/servers-dir)))
    (when (path-exists? sdir)
      (for-each
        (lambda (name)
          (let ((receipt (lsp/read-receipt name)))
            (cond
              ((not receipt)
               (log! 'warn (string-append "LSP: interrupted install of " name
                                          " — run :lsp-install to redo, or delete the directory")))
              ((not (hash-contains? *lsp-servers* name))
               (log! 'warn (string-append "LSP: orphan server " name
                                          " — not in the seeded catalog, run :lsp-uninstall to remove")))
              (else
               (lsp/register-server-languages!
                 name
                 (path-join (lsp/server-dir name) (lsp/receipt-bin receipt))
                 (map (lambda (entry) (cons (car entry) (path-join (lsp/server-dir name) (cdr entry))))
                      (lsp/receipt-env-dirs receipt)))))))
        (call! "stdlib/list-subdirs" sdir)))))

(define-typed-command! "lsp-rescan-servers"
  "Re-scan installed language servers on disk and register any not yet registered."
  (lambda () (lsp/register-installed-servers!)))

(define-typed-command! "lsp-status"
  "Show registered LSP servers and attached buffers' diagnostic counts."
  (lambda (pane) (lsp-show-status! pane)))

(define-typed-command! "lsp-stop"
  "Stop an LSP server: :lsp-stop [language] (default: this buffer's server)."
  (lambda (pane arg) (lsp-stop! (if (string? arg) arg pane))))

(define-typed-command! "lsp-restart"
  "Restart an LSP server: :lsp-restart [language] (default: this buffer's server)."
  (lambda (pane arg) (lsp-restart! (if (string? arg) arg pane))))
