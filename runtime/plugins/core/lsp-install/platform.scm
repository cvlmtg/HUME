;;; core:lsp-install/platform.scm — see README.md.

(provide lsp-install/target lsp-install/windows?)

(define (lsp-install/target)
  (let ((os   (current-os!))
        (arch (target-arch!)))
    (cond ((and (equal? os "macos") (equal? arch "aarch64")) "darwin-arm64")
          ((and (equal? os "macos") (equal? arch "x86_64")) "darwin-x64")
          ((and (equal? os "linux") (equal? arch "x86_64")) "linux-x64")
          ((and (equal? os "windows") (equal? arch "x86_64")) "windows-x64")
          (else #f))))

(define (lsp-install/windows?)
  (equal? (lsp-install/target) "windows-x64"))
