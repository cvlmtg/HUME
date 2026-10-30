;;; core:stdlib/files.scm

;; ── Filesystem helpers (internal) ───────────────────────────────────────────
;; See README.md's "Filesystem".

(define (stdlib/write-file! path content)
  ;; `#:exists 'truncate` — without it, `open-output-file` fails when the file
  ;; already exists.
  (let ([port (open-output-file path #:exists 'truncate)])
    (write-string content port)
    (close-output-port port)))

;;; Idempotent, unlike `remove-dir!` — see README.md's "Filesystem".
(define (stdlib/delete-dir! dir)
  (when (path-exists? dir)
    (remove-dir! dir)))

;;; Idempotent, unlike Steel's own `delete-file!` — see README.md's "Filesystem".
(define (stdlib/delete-file! path)
  (when (path-exists? path)
    (delete-file! path)))

;;; `is-dir?` drops stray files alongside a directory tree — see README.md's "Filesystem".
(define (stdlib/list-entries dir)
  (sort (map file-name (read-dir dir)) string<?))

(define (stdlib/list-subdirs dir)
  (filter (lambda (name) (is-dir? (path-join dir name)))
          (stdlib/list-entries dir)))

(define (stdlib/path-absolute? path)
  (if (equal? (current-os!) "windows")
      (or (starts-with? path "\\\\")
          (and (>= (string-length path) 3)
               (char<=? #\A (char-upcase (string-ref path 0)) #\Z)
               (equal? (string-ref path 1) #\:)
               (let ((sep (string-ref path 2)))
                 (or (equal? sep #\\) (equal? sep #\/)))))
      (starts-with? path "/")))

;; ── Downloads, digests and archives ──────────────────────────────────────────
;; See README.md's "Downloads, digests and archives".

;;; `range` is empty for the whole file, or the offset and size of the bytes to fetch.
(define (stdlib/download! url dest . range)
  (let ((bytes (cond ((null? range) '())
                     ((and (= (length range) 2)
                           (integer? (car range)) (>= (car range) 0)
                           (integer? (cadr range)) (> (cadr range) 0))
                      (list "-r" (string-append (number->string (car range)) "-"
                                                (number->string (+ (car range) (cadr range) -1)))))
                     (else (error (string-append "stdlib/download!: a range is an offset and a "
                                                 "size, got " (to-string range)))))))
    (run-inline-output! "curl" (append (list "-fsSL") bytes (list "-o" dest "--" url)))))

(define (stdlib/sha256-argv path)
  (let ((os (current-os!)))
    (cond ((equal? os "macos") (list "shasum" "-a" "256" path))
          ((equal? os "windows") (list "certutil" "-hashfile" path "SHA256"))
          (else (list "sha256sum" path)))))

(define (stdlib/parse-sha256 stdout)
  (if (equal? (current-os!) "windows")
      (let ((lines (filter (lambda (line) (not (equal? (trim line) "")))
                           (split-many stdout "\n"))))
        (and (> (length lines) 1)
             (apply string-append (split-whitespace (car (cdr lines))))))
      (let ((tokens (split-whitespace stdout)))
        (and (pair? tokens) (car tokens)))))

(define (stdlib/sha256-file path)
  (let* ((argv   (stdlib/sha256-argv path))
         (result (run-capture! (car argv) (cdr argv))))
    (unless (equal? (hash-ref result 'exit) 0)
      (error (string-append "stdlib/verify-sha256!: " (car argv) " failed: "
                            (trim (hash-ref result 'stderr)))))
    (let ((digest (stdlib/parse-sha256 (hash-ref result 'stdout))))
      (unless digest
        (error (string-append "stdlib/verify-sha256!: could not parse " (car argv)
                              " output: " (trim (hash-ref result 'stdout)))))
      (string-downcase digest))))

(define (stdlib/verify-sha256! path expected)
  (let ((expected (string-downcase expected))
        (actual (stdlib/sha256-file path)))
    (unless (equal? actual expected)
      (error (string-append "stdlib/verify-sha256!: sha256 mismatch for '" path
                            "': expected " expected ", got " actual)))))

(struct extracted-tree (files links))

(define (stdlib/tree-entries dir)
  (let walk ((dir dir) (files '()) (links '()))
    (let ((iter (read-dir-iter dir)))
      (let loop ((files files) (links links))
        (let ((entry (read-dir-iter-next! iter)))
          (cond ((not entry) (extracted-tree files links))
                ((read-dir-entry-is-symlink? entry)
                 (loop files (cons (read-dir-entry-path entry) links)))
                ((read-dir-entry-is-dir? entry)
                 (let ((sub (walk (read-dir-entry-path entry) files links)))
                   (loop (extracted-tree-files sub) (extracted-tree-links sub))))
                ((read-dir-entry-is-file? entry)
                 (loop (cons (read-dir-entry-path entry) files) links))
                (else (loop files links))))))))

;;; See README.md's "Downloads, digests and archives" for the Windows order.
(define (stdlib/windows-tar)
  (let ((root (maybe-get-env-var "SystemRoot")))
    (and (Ok? root)
         (let ((tar (path-join (Ok->value root) "System32" "tar.exe")))
           (and (is-file? tar) tar)))))

(define (stdlib/found kind program)
  (and program (cons kind program)))

(define (stdlib/extractor format)
  (cond
    ((not (member format '("tar" "zip")))
     (error (string-append "stdlib: no extractor for '" format "' archives")))
    ((not (equal? (current-os!) "windows"))
     (if (equal? format "zip")
         (stdlib/found 'unzip (find-program "unzip"))
         (stdlib/found 'tar (find-program "tar"))))
    (else
     (or (stdlib/found 'tar (stdlib/windows-tar))
         (and (equal? format "zip") (stdlib/found 'unzip (find-program "unzip")))
         (stdlib/found 'tar (find-program "tar"))))))

(define (stdlib/find-extractor format)
  (let ((extractor (stdlib/extractor format)))
    (and extractor (cdr extractor))))

(define (stdlib/extract-argv format archive dir)
  (let ((extractor (stdlib/extractor format)))
    (unless extractor
      (error (string-append "stdlib/extract-archive!: no program that extracts " format
                            " archives was found")))
    (if (equal? (car extractor) 'unzip)
        (list (cdr extractor) "-o" archive "-d" dir)
        (list (cdr extractor) "-xf" archive "-C" dir))))

;;; Returns the regular files under `dir` after extraction.
(define (stdlib/extract-archive! format archive dir)
  (create-directory! dir)
  (let ((argv (stdlib/extract-argv format archive dir)))
    (run-inline-output! (car argv) (cdr argv)))
  (let* ((entries (stdlib/tree-entries dir))
         (escaping (first-path-outside dir (extracted-tree-links entries))))
    (when escaping
      (error (string-append "stdlib/extract-archive!: the archive has a symlink pointing "
                            "outside " dir ": " escaping)))
    (extracted-tree-files entries)))

;; ── call!-able commands (public API) ────────────────────────────────────────

(define-command! "stdlib/write-file!"
  "Write the given content to the given path, creating or truncating it."
  stdlib/write-file!)

(define-command! "stdlib/delete-dir!"
  "Recursively delete the given directory. Idempotent."
  stdlib/delete-dir!)

(define-command! "stdlib/delete-file!"
  "Delete the file at the given path. Idempotent."
  stdlib/delete-file!)

(define-command! "stdlib/list-entries"
  "Sorted basenames of every entry of the given directory. Raises when the directory cannot be read, a missing one included."
  stdlib/list-entries)

(define-command! "stdlib/list-subdirs"
  "Sorted basenames of the given directory's subdirectories. Raises when the directory cannot be read, a missing one included."
  stdlib/list-subdirs)

(define-command! "stdlib/path-absolute?"
  "Whether the given path names the same place from any working directory."
  stdlib/path-absolute?)

(define-command! "stdlib/download!"
  "Download the given URL to the given file with curl, raising on an HTTP error. An offset and a size after the file fetch only those bytes."
  stdlib/download!)

(define-command! "stdlib/verify-sha256!"
  "Raise unless the file at the given path has the given SHA-256, as hex in any case."
  stdlib/verify-sha256!)

(define-command! "stdlib/find-extractor"
  "The program stdlib/extract-archive! runs for the given archive format (\"tar\" or \"zip\"), or #f when there is none."
  stdlib/find-extractor)

(define-command! "stdlib/extract-archive!"
  "Extract the given archive, of the given format (\"tar\" or \"zip\"), into the given directory, creating it; raise if a symlink in it points outside that directory. Returns the extracted regular files."
  stdlib/extract-archive!)
