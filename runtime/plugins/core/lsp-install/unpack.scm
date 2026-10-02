;;; core:lsp-install/unpack.scm — see README.md.

(require "platform.scm")

(provide lsp-install/unpack-gz! lsp-install/unpack-archive! lsp-install/mark-executable!)

(define lsp-install/chmod-batch 200)

(define (lsp-install/regular-files dir)
  (let walk ((dir dir) (files '()))
    (let ((iter (read-dir-iter dir)))
      (let loop ((files files))
        (let ((entry (read-dir-iter-next! iter)))
          (cond ((not entry) files)
                ((read-dir-entry-is-dir? entry)
                 (loop (walk (read-dir-entry-path entry) files)))
                ((read-dir-entry-is-file? entry)
                 (loop (cons (read-dir-entry-path entry) files)))
                (else (loop files))))))))

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

(define (lsp-install/extract-argv tool archive dir)
  (if (equal? tool "unzip")
      (list "unzip" "-o" archive "-d" dir)
      (list "tar" "-xf" archive "-C" dir)))

(define (lsp-install/unpack-archive! tool archive dir)
  (create-directory! dir)
  (let ((argv (lsp-install/extract-argv tool archive dir)))
    (run-inline-output! (car argv) (cdr argv)))
  (lsp-install/mark-executable! (lsp-install/regular-files dir)))

(define (lsp-install/unpack-gz! archive dest)
  (run-inline-output! "gzip" (list "-d" "-f" archive))
  (let ((decoded (substring archive 0 (- (string-length archive) 3))))
    (unless (equal? decoded dest)
      (rename-file-or-directory! decoded dest))
    (lsp-install/mark-executable! (list dest))))
