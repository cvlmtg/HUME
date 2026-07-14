;;; core:lsp/registration.scm — the LSP server catalog, receipt/path
;;; primitives, and the scan that turns an installed server into a live
;;; registration. `servers.scm` (this plugin's install/uninstall pipeline)
;;; requires this file for its read-side helpers — both live in core:lsp, so
;;; there is no cross-plugin require to route around (see docs/ROADMAP.md
;;; "Plugin namespace isolation").

(require-builtin steel/vectors)
(provide lsp/register-installed-servers! lsp/field lsp/servers-dir lsp/server-dir
         lsp/receipt-path lsp/read-receipt lsp/receipt-bin lsp/receipt-version
         lsp/servers-catalog)

;; ── Server catalog ────────────────────────────────────────────────────────────
;;
;; Hash: name → server-entry fields, the tagged-alist tail from
;; lsp-servers.scm: (languages ...) (command . cmd) (args ...) (settings ...).
;; Exposed read-only via `lsp/servers-catalog` — `servers.scm` needs it to
;; resolve `:lsp-install <lang>` and list `:lsp-servers`; this file needs it
;; to turn a receipt's bin path into a full registration.
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
;;
;; Entries in the catalog are tagged alists — `(key . value)` (a scalar or
;; #(...) vector leaf) or `(key sub…)` (a nested list) — never positional
;; tuples. `car` works on both shapes, which is what makes a single lookup
;; helper possible.

;;; First element of `fields` whose car is `key` (a symbol), or `#f`.
(define (lsp/field fields key)
  (cond ((null? fields) #f)
        ((equal? (car (car fields)) key) (car fields))
        (else (lsp/field (cdr fields) key))))

;; ── Directory entry filter + listing ──────────────────────────────────────────

;;; Return #t if `name` is a valid, traversable directory entry (not "." or "..").
(define (lsp/valid-dir-entry? name)
  (and (not (equal? name "."))
       (not (equal? name ".."))))

;;; Sorted list of basenames in `dir`.
(define (lsp/list-dir dir)
  (sort (map file-name (read-dir dir)) string<?))

;; ── Paths ─────────────────────────────────────────────────────────────────────

(define (lsp/servers-dir) (path-join (data-dir) "servers"))
(define (lsp/server-dir name) (path-join (lsp/servers-dir) name))
(define (lsp/receipt-path name) (path-join (lsp/server-dir name) "receipt.scm"))

;; ── Receipts ──────────────────────────────────────────────────────────────────
;;
;; receipt.scm is the install commit point: pure data
;; `((name . "X") (version . "V") (bin . "relative/bin/path"))`, written by
;; `servers.scm`'s `lsp/install-server!`. A server dir without a readable
;; receipt is an interrupted install (see docs/LSP-INSTALL.md "Installation
;; layout").

;;; Read `name`'s receipt, or `#f` if missing/unreadable (interrupted install).
(define (lsp/read-receipt name)
  (with-handler (lambda (err) #f)
    (call-with-input-file (lsp/receipt-path name) read)))

(define (lsp/receipt-bin receipt) (cdr (lsp/field receipt 'bin)))
(define (lsp/receipt-version receipt) (cdr (lsp/field receipt 'version)))

;; ── Settings conversion ───────────────────────────────────────────────────────
;;
;; Seeded settings are nested alists whose entries take one of three shapes
;; (see docs/LSP-INSTALL.md "Seeded data format"): `(key . scalar)`,
;; `(key . #(elem…))` for a JSON array, or `(key entry…)` for a nested
;; object. `steel_to_json` has no case for a raw vector, so every `#(...)`
;; must become a Steel list before it can reach `#:settings`.

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

;;; Register `name` for every language it serves that isn't registered yet,
;;; with `cmd` as the server command. `args`/`settings` are shared across a
;;; multi-language server; only root markers vary per language (see
;;; docs/LSP-INSTALL.md "Seeded data format"). Skipping an already-registered
;;; language is what makes a mid-session rescan (`:lsp-rescan-servers`, or
;;; the rescan `:lsp-install` runs after installing a server that was already
;;; registered) leave a user's own manual `register-lsp-server!` override
;;; alone instead of last-wins-clobbering it with the catalog default — the
;;; scan only needs to pick up languages nothing has claimed yet. At load
;;; time the registry is empty, so every language passes the filter and gets
;;; registered — matching this being the scan's first pass over the catalog.
;;;
;;; `#:force?` skips that filter and registers every one of `name`'s
;;; languages unconditionally. Needed right after `lsp/install-server!`
;;; queues an `unregister-lsp-server!` for these same languages: that op
;;; applies at end-of-eval, so `lsp-registered-for-language?` still reports
;;; the *pre*-unregister (registered) state for the rest of this eval — the
;;; unfiltered scan would see nothing to do, and the queued unregister would
;;; land with no matching re-registration behind it. Only the install path
;;; (which just issued that unregister itself) should pass `#:force? #t`.
;;;
;;; Same-eval blindness cuts the other way too, unrelated to `#:force?`: a
;;; user's own `register-lsp-server!` placed *before* an eager
;;; `(load-plugin "core:lsp")` in init.scm is a queued op that hasn't
;;; applied yet when this scan's filter runs, so the filter doesn't see it
;;; and the catalog default gets queued right behind it. This is not a bug
;;; to fix here, though — `register-lsp-server!` is last-wins over the
;;; *order ops are queued in*, not over what the live registry says at scan
;;; time, so simply queuing the override *after* `load-plugin` (rather than
;;; before) is a complete fix with no code change: the override then queues
;;; strictly after the scan's op and always wins. Documented as the required
;;; ordering for eager loading in README.md and
;;; user-manual/docs/lsp.md#registering-a-language-server. The lazy
;;; `(declare-plugin "core:lsp")` form is unaffected either way — its scan
;;; runs on activation, always after init.scm has already finished.
(define (lsp/register-server-languages! name cmd #:force? [force? #f])
  (let* ((fields   (hash-ref *lsp-servers* name))
         (all-langs (cdr (lsp/field fields 'languages)))
         (langs    (if force?
                       all-langs
                       (filter (lambda (lang-entry) (not (lsp-registered-for-language? (car lang-entry))))
                               all-langs)))
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
;; naming a seeded server), no subprocess, no network. `.install-lock`
;; (the cross-process install lock sentinel file `servers.scm` acquires
;; around install/uninstall) lives directly under `servers-dir` alongside
;; the per-server subdirectories — excluded here so a lock left behind by a
;; crash (or present during a legitimate concurrent install elsewhere) is
;; never misread as an interrupted or orphan server.
;;
;; Runs at plugin load, or at lazy activation. Either way,
;; `apply_pending_lsp_server_reg` (hume-editor/src/editor/lsp/registry.rs)
;; sweeps already-open buffers on every registration once it's actually
;; applied — at eager plugin load that happens synchronously (registrations
;; queued during init.scm are flushed once at the end of init); a *lazy*
;; activation applies its queued registrations immediately too
;; (`activate_and_register`, hume-editor/src/editor/mappings/lazy.rs), so the
;; buffer that triggered the activation attaches in that same call. Also
;; exposed as the `lsp-rescan-servers` command, and called directly by
;; `servers.scm`'s install/uninstall commands right after they mutate disk —
;; no cross-plugin notify needed, both live here.
;;
;; This is the *only* registrar for managed (installed) servers, and the
;; above is the *only* place it runs — load or lazy activation. Consequence:
;; declaring core:lsp solely on its own downstream `on-lsp-attach` event
;; self-deadlocks — nothing is registered until this runs, so nothing
;; attaches, so the event that would trigger activation never fires. See
;; docs/LSP-INSTALL.md "Registration model".
;;;
;;; `#:force-name` names a single server (typically the one `servers.scm`'s
;;; install path just installed) whose languages register unconditionally —
;;; see `lsp/register-server-languages!`'s `#:force?` doc for why. Every
;;; other server in the scan still gets the normal no-clobber filter.
(define (lsp/register-installed-servers! #:force-name [force-name #f])
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
                 #:force? (equal? name force-name))))))
        (filter (lambda (name) (and (lsp/valid-dir-entry? name) (not (equal? name ".install-lock"))))
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
