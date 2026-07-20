;;; core:steel-server

(define (steel-server/register!)
  (unless (lsp-registered-for-language? "scheme")
    (register-lsp-server! "scheme"
                          #:command "steel-language-server"
                          #:root-markers '("cog.scm"))))

(define-command! "steel-server-install"
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
