;;; core:steel-server

;;; Directory of the generated host-globals file steel-language-server reads
;;; (see lsp-home/hume-globals.scm) — #f when the runtime dir is unavailable
;;; or the file wasn't staged. The existence check is load-bearing, not
;;; defensive: the server panics at startup if STEEL_LSP_HOME names a
;;; missing directory (it `read_dir`s it unconditionally, no fallback).
(define (steel-server/lsp-home)
  (let ([rt (runtime-dir)])
    (and rt
         (let ([dir (path-join rt "plugins" "core" "steel-server" "lsp-home")])
           (and (path-exists? (path-join dir "hume-globals.scm")) dir)))))

(define (steel-server/register!)
  (unless (lsp-registered-for-language? "scheme")
    (let ([home (steel-server/lsp-home)])
      (unless home
        (log! 'warn "steel-server: host-globals dir missing — HUME builtins will be flagged as unknown identifiers"))
      (register-lsp-server! "scheme"
                            #:command "steel-language-server"
                            #:root-markers '("cog.scm")
                            #:env (if home (list (cons "STEEL_LSP_HOME" home)) '())))))

(define-typed-command! "steel-server-install"
  "Install steel-language-server with cargo and register it for scheme buffers."
  (lambda ()
    (if (which "steel-language-server")
        (log! 'info "steel-server: steel-language-server is already installed")
        (begin
          (unless (which "cargo")
            (error "steel-server-install: requires 'cargo' on $PATH — install Rust from https://rustup.rs"))
          (log! 'info "steel-server: running cargo install steel-language-server...")
          (run-inline-output! "cargo" '("install" "steel-language-server"))
          (unless (which "steel-language-server")
            (error "steel-server-install: cargo install succeeded but 'steel-language-server' is not on $PATH — add ~/.cargo/bin to PATH and run :steel-server-install again"))))
    (steel-server/register!))
  #:inline-output #t)

(if (which "steel-language-server")
    (steel-server/register!)
    (log! 'warn "steel-server: 'steel-language-server' not found on $PATH — run :steel-server-install"))
