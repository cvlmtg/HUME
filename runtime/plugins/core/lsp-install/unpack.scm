;;; core:lsp-install/unpack.scm — see README.md.

(require "platform.scm")

(provide lsp-install/unpack-gz! lsp-install/unpack-archive! lsp-install/unpack-tool
         lsp-install/mark-executable!)

(define lsp-install/chmod-batch 200)

(define (lsp-install/regular-files dir)
  (let ((iter (read-dir-iter dir)))
    (let loop ((files '()))
      (let ((entry (read-dir-iter-next! iter)))
        (cond ((not entry) files)
              ((read-dir-entry-is-dir? entry)
               (loop (append (lsp-install/regular-files (read-dir-entry-path entry)) files)))
              ((read-dir-entry-is-file? entry)
               (loop (cons (read-dir-entry-path entry) files)))
              (else (loop files)))))))

(define (lsp-install/take lst n)
  (if (or (null? lst) (= n 0))
      '()
      (cons (car lst) (lsp-install/take (cdr lst) (- n 1)))))

(define (lsp-install/drop lst n)
  (if (or (null? lst) (= n 0))
      lst
      (lsp-install/drop (cdr lst) (- n 1))))

(define (lsp-install/mark-executable! files)
  (unless (or (lsp-install/windows?) (null? files))
    (run-inline-output! "chmod" (cons "755" (lsp-install/take files lsp-install/chmod-batch)))
    (lsp-install/mark-executable! (lsp-install/drop files lsp-install/chmod-batch))))

(define (lsp-install/unpack-tool fmt)
  (if (and (equal? fmt 'zip) (not (lsp-install/windows?)))
      "unzip"
      "tar"))

(define (lsp-install/extract-argv fmt archive dir)
  (if (equal? (lsp-install/unpack-tool fmt) "unzip")
      (list "unzip" "-o" archive "-d" dir)
      (list "tar" "-xf" archive "-C" dir)))

(define (lsp-install/unpack-archive! fmt archive dir bin)
  (create-directory! dir)
  (let ((argv (lsp-install/extract-argv fmt archive dir)))
    (run-inline-output! (car argv) (cdr argv)))
  (let ((files (lsp-install/regular-files dir)))
    (unless (or (lsp-install/windows?) (member (path-join dir bin) files))
      (error (string-append "lsp-install/unpack-archive!: extracted archive is missing expected binary: "
                            bin)))
    (lsp-install/mark-executable! files)))

(define (lsp-install/unpack-gz! archive dest)
  (run-inline-output! "gzip" (list "-d" "-f" archive))
  (let ((decoded (substring archive 0 (- (string-length archive) 3))))
    (unless (equal? decoded dest)
      (rename-file-or-directory! decoded dest))
    (lsp-install/mark-executable! (list dest))))
