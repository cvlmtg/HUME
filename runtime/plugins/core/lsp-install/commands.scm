;;; core:lsp-install/commands.scm — see README.md.

(require "lib/catalog.scm")
(require "lib/receipts.scm")
(require "lib/register.scm")
(require "lib/blocker.scm")
(require "lib/source-catalog.scm")
(require "lib/install.scm")
(require "lib/lock.scm")

(define (lsp-install/install! name)
  (let ((installed? (lsp-install/with-lock! (string-append "install " name)
                      (lambda ()
                        (print! (string-append "LSP: installing " name "..."))
                        (lsp-install/install-server! name)
                        (print! (string-append "LSP: installed " name))))))
    (lsp-install/register-installed-servers!)
    (when (and (not installed?) (lsp-install/read-receipt name))
      (log! 'info (string-append "LSP: kept the installed " name)))))

;;; See docs/install.md's "Installing a server".
(define-command! "lsp-install/install-server"
  "Install the server `name` over its existing install, once its client has stopped."
  lsp-install/install!
  #:inline-output #t)

(define (lsp-install/install-or-report! name)
  (let* ((receipt (lsp-install/read-receipt name))
         (source  (lsp-install/source name)))
    (cond
      ((and receipt source
            (equal? (lsp-install/receipt-version receipt) (lsp-install/ref source 'version)))
       (log! 'info (string-append "LSP: " name " already installed (v"
                                  (lsp-install/receipt-version receipt) ") — up to date"))
       (lsp-install/register-installed-servers!))
      (receipt
       (unregister-lsp-server! name)
       (after! 0 (lambda () (call! "lsp-install/install-server" name))))
      (else (lsp-install/install! name)))))

(register-completion-source! "lsp:install-targets"
  (lambda (id input cursor)
    (let ((servers (lsp-install/source-names)))
      (completion-emit! id
        (append servers
                (filter (lambda (lang) (not (member lang servers)))
                        (lsp-install/language-names))))))
  #:target 'minibuf #:match 'string)

(define-typed-command! "lsp-install"
  "Download and verify a language server, then register it: :lsp-install [language|server] (default: the current buffer's language)."
  (lambda (pane arg)
    (if (and (string? arg) (lsp-install/seeded-server? arg))
        (lsp-install/install-or-report! arg)
        (let ((lang (call! "stdlib/resolve-lang-arg" pane "lsp-install" arg)))
          (cond
            ((not lang) (begin))
            ((not (lsp-install/seeded-language? lang))
             (log! 'error (string-append "lsp-install: no language or server named \"" lang "\" is seeded")))
            (else
             (lsp-install/install-or-report! (lsp-install/primary lang)))))))
  #:inline-output #t #:complete "lsp:install-targets")

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
       (log! 'error "lsp-uninstall: requires a server name, e.g. :lsp-uninstall rust-analyzer"))
      ((not (safe-path-segment? arg))
       (log! 'error (string-append "lsp-uninstall: invalid server name: " arg)))
      (else
        (let* ((name arg)
               (dir  (lsp-install/server-dir name)))
          (when (lsp-install/seeded-server? name)
            (unregister-lsp-server! name))
          (if (path-exists? dir)
              (begin
                (log! 'info (string-append "LSP: shutting down and removing " name "..."))
                (after! 0 (lambda ()
                           (when (lsp-install/with-lock! (string-append "uninstall " name)
                                   (lambda () (call! "stdlib/delete-dir!" dir)))
                             (log! 'info (string-append "LSP: removed " name))))))
              (log! 'info (string-append "LSP: nothing to uninstall for " name)))))))
  #:complete "lsp:servers")

(define-typed-command! "lsp-catalog"
  "Show the LSP server catalog: languages, seeded version, and install status."
  (lambda (pane)
    (call! "stdlib/show-report!" pane "lsp-catalog"
      (list
        (call! "stdlib/report-section" "LSP servers"
          (map
            (lambda (name)
              (let* ((receipt (lsp-install/read-receipt name))
                     (source  (lsp-install/source name))
                     (langs   (map (lambda (lang)
                                     (if (equal? (lsp-install/primary lang) name)
                                         lang
                                         (string-append lang " (secondary)")))
                                   (lsp-install/server-languages name)))
                     (status
                       (cond
                         (receipt
                          (let ((installed (lsp-install/receipt-version receipt))
                                (seeded    (if source (lsp-install/ref source 'version) #f)))
                            (if (and seeded (not (equal? installed seeded)))
                                (string-append "installed v" installed " — update available (v" seeded ")")
                                (string-append "installed v" installed))))
                         (else
                          (let ((blocker (lsp-install/install-blocker name)))
                            (if blocker blocker "not installed"))))))
                (string-append name " [" (string-join langs ", ") "]: " status)))
            (lsp-install/server-names))
          #f)))))
