;;; core:lsp/registration.scm — the LSP server catalog, receipt/path
;;; primitives, and the scan that turns an installed server into a live
;;; registration. `servers.scm` requires this file for its read-side helpers.

(provide lsp/register-installed-servers! lsp/field lsp/servers-dir lsp/server-dir
         lsp/receipt-path lsp/read-receipt lsp/receipt-bin lsp/receipt-version
         lsp/servers-catalog)

;; ── Server catalog ────────────────────────────────────────────────────────────
;; Hash: name → server-entry fields, the tagged-alist tail from
;; lsp-servers.scm: (languages ...) (command . cmd) (args ...) (config . …).
;; See README "Server config delivery" for how `config` reaches the server.
(define *lsp-servers* (hash))

;;; Register one lsp-servers.scm entry: `(name field...)`.
(define (lsp/declare-server! entry)
  (set! *lsp-servers* (hash-insert *lsp-servers* (car entry) (cdr entry))))

(for-each lsp/declare-server!
  (call-with-input-file
    (path-join (runtime-dir) "scheme" "lsp-servers.scm")
    read))

;;; The seeded server catalog: name → server-entry fields. Read-only accessor
;;; — callers must not mutate the returned hash.
(define (lsp/servers-catalog) *lsp-servers*)

;; ── Field access ──────────────────────────────────────────────────────────────
;; Catalog entries are tagged alists — `(key . value)` or `(key sub…)`, never
;; positional tuples — `car` works on both shapes.

;;; First element of `fields` whose car is `key` (a symbol), or `#f`.
(define (lsp/field fields key)
  (cond ((null? fields) #f)
        ((equal? (car (car fields)) key) (car fields))
        (else (lsp/field (cdr fields) key))))

;; ── Directory entry filter + listing ──────────────────────────────────────────

;;; #t if `name` is a server subdirectory of `parent` — filters out stray
;;; non-directory entries `read-dir` returns alongside them. The systematic
;;; case is `.install-lock` (`servers.scm`'s cross-process install sentinel,
;;; created directly in this same directory); `.DS_Store` is incidental and
;;; macOS-only.
(define (lsp/valid-dir-entry? parent name)
  (is-dir? (path-join parent name)))

;;; Sorted list of basenames in `dir`.
(define (lsp/list-dir dir)
  (sort (map file-name (read-dir dir)) string<?))

;; ── Paths ─────────────────────────────────────────────────────────────────────

(define (lsp/servers-dir) (path-join (data-dir) "servers"))
(define (lsp/server-dir name) (path-join (lsp/servers-dir) name))
(define (lsp/receipt-path name) (path-join (lsp/server-dir name) "receipt.scm"))

;; ── Receipts ──────────────────────────────────────────────────────────────────
;; receipt.scm is the install commit point: pure data
;; `((name . "X") (version . "V") (bin . "relative/bin/path"))`. A server dir
;; without a readable receipt is an interrupted install.

;;; Read `name`'s receipt, or `#f` if missing/unreadable (interrupted install).
(define (lsp/read-receipt name)
  (with-handler (lambda (err) #f)
    (call-with-input-file (lsp/receipt-path name) read)))

(define (lsp/receipt-bin receipt) (cdr (lsp/field receipt 'bin)))
(define (lsp/receipt-version receipt) (cdr (lsp/field receipt 'version)))

;; ── Registration ──────────────────────────────────────────────────────────────

;;; Register `name` for every language it serves that isn't registered yet,
;;; with `cmd` as the server command. Skipping an already-registered language
;;; is what lets a mid-session rescan leave a user's own manual
;;; `register-lsp-server!` alone instead of last-wins-clobbering it — see
;;; README "Usage". `lsp-registered-for-language?` reads through the
;;; same-eval pending op queue, so this filter is always correct in queue
;;; order regardless of load order.
(define (lsp/register-server-languages! name cmd)
  (let* ((fields   (hash-ref *lsp-servers* name))
         (langs    (filter (lambda (lang-entry) (not (lsp-registered-for-language? (car lang-entry))))
                            (cdr (lsp/field fields 'languages))))
         (args     (cdr (lsp/field fields 'args)))
         ;; `(config . "json")` or empty-tail `(config)` — `cdr` gives the
         ;; JSON string or '() respectively.
         (config-json (cdr (lsp/field fields 'config)))
         (config (if (null? config-json) #f (json-parse config-json))))
    (for-each
      (lambda (lang-entry)
        ;; Delivered both ways, as Helix does — see README "Server config
        ;; delivery".
        (register-lsp-server! (car lang-entry)
                               #:command cmd
                               #:args args
                               #:root-markers (cdr lang-entry)
                               #:init-options config
                               #:settings config))
      langs)))

;; ── Startup server registration ───────────────────────────────────────────────
;; Passive: registers already-installed servers only, no subprocess, no
;; network. Runs at plugin load, lazy activation, and after servers.scm's
;; install/uninstall mutate disk; also exposed as `lsp-rescan-servers`.
;; `apply_pending_lsp_server_reg` (hume-editor/src/editor/lsp/registry.rs)
;; sweeps already-open buffers on every registration this queues. The *only*
;; registrar for managed servers — see README Caveat for the self-deadlock
;; this implies for a manifest keyed only on `on-lsp-attach`.
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
                 (path-join (lsp/server-dir name) (lsp/receipt-bin receipt)))))))
        (filter (lambda (name) (lsp/valid-dir-entry? sdir name))
                (lsp/list-dir sdir))))))

(define-command! "lsp-rescan-servers"
  "Re-scan installed language servers on disk and register any not yet registered."
  (lambda () (lsp/register-installed-servers!)))

(define-command! "lsp-status"
  "Show registered LSP servers and attached buffers' diagnostic counts."
  (lambda () (lsp-show-status!)))

(define-command! "lsp-stop"
  "Stop an LSP server: :lsp-stop [language] (default: focused buffer's server)."
  (lambda (arg) (lsp-stop! (if (string? arg) arg #f))))

(define-command! "lsp-restart"
  "Restart an LSP server: :lsp-restart [language] (default: focused buffer's server)."
  (lambda (arg) (lsp-restart! (if (string? arg) arg #f))))
