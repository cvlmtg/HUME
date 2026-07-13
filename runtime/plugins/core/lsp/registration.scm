;;; core:lsp/registration.scm — startup registration of PLUM-installed LSP
;;; servers. Self-contained: loads the seeded catalog and reads receipts
;;; itself rather than requiring anything from core:plum — plugins never
;;; require each other's modules (see docs/ROADMAP.md "Plugin namespace
;;; isolation"). core:plum only installs servers to disk (see
;;; plum/servers.scm); this file is what turns an installed server into a
;;; live registration. A few small helpers below are intentional twins of
;;; ones in plum/servers.scm and plum/lib.scm — kept in sync by hand, not
;;; shared, since there is no cross-plugin require to share them through.

(require-builtin steel/vectors)
(provide lsp/register-installed-servers!)

;; ── Server catalog ────────────────────────────────────────────────────────────
;;
;; Hash: name → server-entry fields, the tagged-alist tail from
;; lsp-servers.scm: (languages ...) (command . cmd) (args ...) (settings ...).
;; Twin of plum/servers.scm's *plum-lsp-servers* — plum needs its own copy to
;; resolve `:lsp-install <lang>` and list `:lsp-servers`; this plugin only
;; needs it to turn a receipt's bin path into a full registration.
(define *lsp-servers* (hash))

;;; Register one lsp-servers.scm entry: `(name field...)`.
(define (lsp/declare-server! entry)
  (set! *lsp-servers* (hash-insert *lsp-servers* (car entry) (cdr entry))))

(for-each lsp/declare-server!
  (call-with-input-file
    (path-join (runtime-dir) "scheme" "lsp-servers.scm")
    read))

;; ── Field access ──────────────────────────────────────────────────────────────
;;
;; Entries in the catalog are tagged alists — `(key . value)` (a scalar or
;; #(...) vector leaf) or `(key sub…)` (a nested list) — never positional
;; tuples. `car` works on both shapes, which is what makes a single lookup
;; helper possible. Twin of plum/field (plum/servers.scm).

;;; First element of `fields` whose car is `key` (a symbol), or `#f`.
(define (lsp/field fields key)
  (cond ((null? fields) #f)
        ((equal? (car (car fields)) key) (car fields))
        (else (lsp/field (cdr fields) key))))

;; ── Directory entry filter + listing ──────────────────────────────────────────
;; Twins of plum/valid-dir-entry? and plum/list-dir (plum/lib.scm).

;;; Return #t if `name` is a valid, traversable directory entry (not "." or "..").
(define (lsp/valid-dir-entry? name)
  (and (not (equal? name "."))
       (not (equal? name ".."))))

;;; Sorted list of basenames in `dir`.
(define (lsp/list-dir dir)
  (sort (map file-name (read-dir dir)) string<?))

;; ── Paths ─────────────────────────────────────────────────────────────────────
;; Twins of plum/servers-dir / plum/server-dir / plum/receipt-path.

(define (lsp/servers-dir) (path-join (data-dir) "servers"))
(define (lsp/server-dir name) (path-join (lsp/servers-dir) name))
(define (lsp/receipt-path name) (path-join (lsp/server-dir name) "receipt.scm"))

;; ── Receipts ──────────────────────────────────────────────────────────────────
;;
;; receipt.scm is the install commit point: pure data
;; `((name . "X") (version . "V") (bin . "relative/bin/path"))`, written by
;; `plum/install-server!`. A server dir without a readable receipt is an
;; interrupted install (see docs/LSP-INSTALL.md "Installation layout").
;; Twins of plum/read-receipt / plum/receipt-version / plum/receipt-bin.

;;; Read `name`'s receipt, or `#f` if missing/unreadable (interrupted install).
(define (lsp/read-receipt name)
  (with-handler (lambda (err) #f)
    (call-with-input-file (lsp/receipt-path name) read)))

(define (lsp/receipt-bin receipt) (cdr (lsp/field receipt 'bin)))

;; ── Settings conversion ───────────────────────────────────────────────────────
;;
;; Seeded settings are nested alists whose entries take one of three shapes
;; (see docs/LSP-INSTALL.md "Seeded data format"): `(key . scalar)`,
;; `(key . #(elem…))` for a JSON array, or `(key entry…)` for a nested
;; object. `steel_to_json` has no case for a raw vector, so every `#(...)`
;; must become a Steel list before it can reach `#:settings`. Twin of
;; plum/vector->steel-list + plum/settings->hash.

;;; Convert a #(...) vector into a Steel list.
(define (lsp/vector->steel-list v)
  (let loop ((i (- (vector-length v) 1)) (acc '()))
    (if (< i 0) acc (loop (- i 1) (cons (vector-ref v i) acc)))))

;;; Convert a settings entry list into a Steel hash suitable for `#:settings`.
(define (lsp/settings->hash entries)
  (let loop ((entries entries) (h (hash)))
    (cond
      ((null? entries) h)
      ((not (pair? (car entries)))
       (error (string-append "lsp/settings->hash: malformed settings entry: "
                             (to-string (car entries)))))
      (else
       (let* ((entry (car entries))
              (key   (car entry))
              (value (if (list? entry)
                         (lsp/settings->hash (cdr entry))
                         (let ((v (cdr entry)))
                           (if (vector? v) (lsp/vector->steel-list v) v)))))
         (loop (cdr entries) (hash-insert h key value)))))))

;; ── Registration ──────────────────────────────────────────────────────────────

;;; Register `name` for every language it serves, with `cmd` as the server
;;; command. `args`/`settings` are shared across a multi-language server;
;;; only root markers vary per language (see docs/LSP-INSTALL.md "Seeded
;;; data format").
(define (lsp/register-server-languages! name cmd)
  (let* ((fields   (hash-ref *lsp-servers* name))
         (langs    (cdr (lsp/field fields 'languages)))
         (args     (cdr (lsp/field fields 'args)))
         (settings-entries (cdr (lsp/field fields 'settings)))
         (settings (if (null? settings-entries) #f (lsp/settings->hash settings-entries))))
    (for-each
      (lambda (lang-entry)
        (register-lsp-server! (car lang-entry)
                               #:command cmd
                               #:args args
                               #:root-markers (cdr lang-entry)
                               #:settings settings))
      langs)))

;; ── Startup server registration ───────────────────────────────────────────────
;;
;; Passive: registers already-installed servers only (a readable receipt
;; naming a seeded server), no subprocess, no network. `.install-lock` (PLUM's
;; cross-process install lock sentinel file) lives directly under
;; `servers-dir` alongside the per-server subdirectories — excluded here so a
;; lock left behind by a crash (or present during a legitimate concurrent
;; install elsewhere) is never misread as an interrupted or orphan server.
;;
;; Runs at plugin load, or at lazy activation. Either way,
;; `apply_pending_lsp_server_reg` (hume-editor/src/editor/lsp/registry.rs)
;; sweeps already-open buffers on every registration once it's actually
;; applied — at eager plugin load that happens synchronously (registrations
;; queued during init.scm are flushed once at the end of init); a *lazy*
;; activation only queues them, and they aren't applied until the next
;; effects-draining point (a hook with a registered handler, or the next
;; command dispatch) — see docs/ROADMAP.md's "Lazy-activation queued effects"
;; open question. Also exposed as the `lsp-rescan-servers` command so PLUM
;; can trigger a rescan right after an install without this plugin requiring
;; anything from PLUM's module.
(define (lsp/register-installed-servers!)
  (let ((sdir (lsp/servers-dir)))
    (when (path-exists? sdir)
      (for-each
        (lambda (name)
          (let ((receipt (lsp/read-receipt name)))
            (cond
              ((not receipt)
               (log! 'warn (string-append "LSP: interrupted install of " name
                                          " — run :lsp-install (core:plum) to redo, or delete the directory")))
              ((not (hash-contains? *lsp-servers* name))
               (log! 'warn (string-append "LSP: orphan server " name
                                          " — not in the seeded catalog, run :lsp-uninstall (core:plum) to remove")))
              (else
               (lsp/register-server-languages!
                 name
                 (path-join (lsp/server-dir name) (lsp/receipt-bin receipt)))))))
        (filter (lambda (name) (and (lsp/valid-dir-entry? name) (not (equal? name ".install-lock"))))
                (lsp/list-dir sdir))))))

(define-command! "lsp-rescan-servers"
  "Re-scan installed language servers on disk and register any not yet registered."
  (lambda () (lsp/register-installed-servers!)))
