;;; core:lsp-install/commands.scm — see README.md.

(require "catalog.scm")
(require "receipts.scm")
(require "register.scm")
(require "install.scm")
(require "lock.scm")

(define *lsp-install-hinted-languages* (hash))

(define (lsp-install/install-or-report! name)
  (let* ((receipt (lsp-install/read-receipt name))
         (source  (if (hash-contains? (lsp-install/sources-catalog) name)
                      (hash-ref (lsp-install/sources-catalog) name)
                      #f)))
    (if (and receipt source
             (equal? (lsp-install/receipt-version receipt) (cdr (lsp-install/field source 'version))))
        (begin
          (log! 'info (string-append "LSP: " name " already installed (v"
                                     (lsp-install/receipt-version receipt) ") — up to date"))
          (lsp-install/register-installed-servers!))
        (let ((had-dir? (path-exists? (lsp-install/server-dir name))))
          (if (lsp-install/with-lock! (string-append "install " name)
                (lambda ()
                  (log! 'info (string-append "LSP: installing " name "..."))
                  (lsp-install/install-server! name)))
              (lsp-install/register-installed-servers!)
              (when had-dir?
                (log! 'info "LSP: if the server was running it has now been shut down — run :lsp-install again")))))))

(register-completion-source! "lsp:languages"
  (lambda (id input cursor)
    (completion-emit! id (hash-keys->list (lsp-install/lang->server))))
  #:target 'minibuf #:match 'string)

(define-typed-command! "lsp-install"
  "Download and verify the language server for a language (default: the current buffer's language), then register it."
  (lambda (pane arg)
    (let ((lang (call! "stdlib/resolve-lang-arg" pane "lsp-install" arg)))
      (cond
        ((not lang) (begin))
        ((not (hash-contains? (lsp-install/lang->server) lang))
         (log! 'info (string-append "lsp-install: no language server is seeded for \"" lang "\"")))
        (else
         (lsp-install/install-or-report! (hash-ref (lsp-install/lang->server) lang))))))
  #:inline-output #t #:complete "lsp:languages")

(register-completion-source! "lsp:servers"
  (lambda (id input cursor)
    (let ((sdir (lsp-install/servers-dir)))
      (completion-emit! id
        (if (path-exists? sdir)
            (call! "stdlib/list-subdirs" sdir)
            '()))))
  #:target 'minibuf #:match 'string)

(define-typed-command! "lsp-uninstall"
  "Shut down and remove an installed language server by name."
  (lambda (pane arg)
    (cond
      ((not (string? arg))
       (log! 'info "lsp-uninstall: requires a server name, e.g. :lsp-uninstall rust-analyzer"))
      ((not (eq? #t (call! "stdlib/safe-path-segment?" arg)))
       (log! 'warn (string-append "lsp-uninstall: invalid server name: " arg)))
      (else
        (let* ((name arg)
               (dir  (lsp-install/server-dir name)))
          (when (hash-contains? (lsp-install/servers-catalog) name)
            (for-each (lambda (lang-entry) (unregister-lsp-server! (car lang-entry)))
                      (cdr (lsp-install/field (hash-ref (lsp-install/servers-catalog) name) 'languages))))
          (if (path-exists? dir)
              (begin
                (log! 'info (string-append "LSP: shutting down and removing " name "..."))
                (after! 0 (lambda ()
                           (when (lsp-install/with-lock! (string-append "uninstall " name)
                                   (lambda () (call! "stdlib/delete-dir!" dir)))
                             (log! 'info (string-append "LSP: removed " name))))))
              (log! 'info (string-append "LSP: nothing to uninstall for " name)))))))
  #:complete "lsp:servers")

(define-typed-command! "lsp-servers"
  "Log the LSP server catalog: languages, seeded version, and install status."
  (lambda ()
    (for-each
      (lambda (name)
        (let* ((receipt (lsp-install/read-receipt name))
               (source  (if (hash-contains? (lsp-install/sources-catalog) name)
                            (hash-ref (lsp-install/sources-catalog) name)
                            #f))
               (langs   (map car (cdr (lsp-install/field (hash-ref (lsp-install/servers-catalog) name) 'languages))))
               (status
                 (cond
                   (receipt
                    (let ((installed (lsp-install/receipt-version receipt))
                          (seeded    (if source (cdr (lsp-install/field source 'version)) #f)))
                      (if (and seeded (not (equal? installed seeded)))
                          (string-append "installed v" installed " — update available (v" seeded ")")
                          (string-append "installed v" installed))))
                   (else
                    (let ((blocker (lsp-install/install-blocker name)))
                      (if blocker blocker "not installed"))))))
          (displayln (string-append name " [" (string-join langs ", ") "]: " status))))
      (hash-keys->list (lsp-install/servers-catalog)))
    (log! 'info (string-append "LSP: " (number->string (length (hash-keys->list (lsp-install/servers-catalog))))
                               " seeded servers")))
  #:inline-output #t)

;; ── Discovery hint ────────────────────────────────────────────────────────────

(register-hook! 'on-language-set
  (lambda (pane lang)
    (when (and (not (equal? lang "")) (not (hash-contains? *lsp-install-hinted-languages* lang)))
      (set! *lsp-install-hinted-languages* (hash-insert *lsp-install-hinted-languages* lang #t))
      (when (hash-contains? (lsp-install/lang->server) lang)
        (let* ((name    (hash-ref (lsp-install/lang->server) lang))
               (blocker (lsp-install/install-blocker name)))
          (when (and (not blocker)
                     (not (lsp-registered-for-language? lang))
                     (not (lsp-install/read-receipt name)))
            (log! 'warn (string-append "LSP: language server '" name
                                       "' is available for " lang " — run :lsp-install"))))))))

(define-typed-command! "lsp-rescan-servers"
  "Re-scan installed language servers on disk and register any not yet registered."
  (lambda () (lsp-install/register-installed-servers!)))
