;;; core:lsp-install/unpack.scm — see README.md.

(require "platform.scm")

(provide lsp-install/unpack-gz! lsp-install/mark-executable!)

(define lsp-install/chmod-batch 200)

(define (lsp-install/take lst n)
  (if (or (null? lst) (= n 0))
      '()
      (cons (car lst) (lsp-install/take (cdr lst) (- n 1)))))

(define (lsp-install/drop lst n)
  (if (or (null? lst) (= n 0))
      lst
      (lsp-install/drop (cdr lst) (- n 1))))

(define (lsp-install/mark-executable! files)
  (unless (or lsp-install/windows? (null? files))
    (run-inline-output! "chmod" (cons "755" (lsp-install/take files lsp-install/chmod-batch)))
    (lsp-install/mark-executable! (lsp-install/drop files lsp-install/chmod-batch))))

(define (lsp-install/unpack-gz! archive dest)
  (run-inline-output! "gzip" (list "-d" "-f" archive))
  (let ((decoded (substring archive 0 (- (string-length archive) 3))))
    (unless (equal? decoded dest)
      (rename-file-or-directory! decoded dest))
    (lsp-install/mark-executable! (list dest))))
