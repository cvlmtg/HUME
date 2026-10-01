;;; core:lsp-install/sha256.scm — see README.md.

(require "platform.scm")

(provide lsp-install/sha256-file)

(define (lsp-install/sha256-argv path)
  (let ((os (current-os!)))
    (cond ((equal? os "macos") (list "shasum" "-a" "256" path))
          ((equal? os "windows") (list "certutil" "-hashfile" path "SHA256"))
          (else (list "sha256sum" path)))))

(define (lsp-install/non-blank-lines text)
  (filter (lambda (line) (not (equal? (trim line) "")))
          (split-many text "\n")))

(define (lsp-install/parse-sha256 stdout)
  (if (lsp-install/windows?)
      (let ((lines (lsp-install/non-blank-lines stdout)))
        (and (> (length lines) 1)
             (apply string-append (split-whitespace (car (cdr lines))))))
      (let ((tokens (split-whitespace stdout)))
        (and (pair? tokens) (car tokens)))))

(define (lsp-install/sha256-file path)
  (let* ((argv   (lsp-install/sha256-argv path))
         (result (run-capture! (car argv) (cdr argv))))
    (unless (equal? (hash-ref result 'exit) 0)
      (error (string-append "lsp-install/sha256-file: " (car argv) " failed: "
                            (trim (hash-ref result 'stderr)))))
    (let ((digest (lsp-install/parse-sha256 (hash-ref result 'stdout))))
      (unless digest
        (error (string-append "lsp-install/sha256-file: could not parse " (car argv)
                              " output: " (trim (hash-ref result 'stdout)))))
      (string-downcase digest))))
