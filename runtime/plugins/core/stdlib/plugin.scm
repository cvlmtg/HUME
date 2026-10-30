;;; core:stdlib

;; ── Selection helpers (internal) ────────────────────────────────────────────
;; Over buffer-selections' entries — see README.md's "Selections".

(define (stdlib/primary-selection sels)
  (and sels
       (let loop ((sels sels))
         (cond
           ((null? sels) #f)
           ((selection-primary? (car sels)) (car sels))
           (else (loop (cdr sels)))))))

(define (stdlib/single-selection? sels)
  (and sels (= (length sels) 1)))

(define (stdlib/all-single-char? sels)
  (and sels
       (let loop ((sels sels))
         (cond
           ((null? sels) #t)
           ((= (selection-anchor (car sels)) (selection-head (car sels)))
            (loop (cdr sels)))
           (else #f)))))

(define (stdlib/cursor-char-offset sels)
  (let ((primary (stdlib/primary-selection sels)))
    (and primary (selection-head primary))))

;; ── Filesystem + list-search helpers (internal) ─────────────────────────────

(define (stdlib/find pred? lst)
  (cond ((null? lst) #f)
        ((pred? (car lst)) (car lst))
        (else (stdlib/find pred? (cdr lst)))))

;;; See README.md's "Filesystem and list search".
(define (stdlib/concat lists)
  (reverse (foldl (lambda (lst acc) (foldl cons acc lst)) '() lists)))

(define (stdlib/write-file! path content)
  ;; `#:exists 'truncate` — without it, `open-output-file` fails when the file
  ;; already exists.
  (let ([port (open-output-file path #:exists 'truncate)])
    (write-string content port)
    (close-output-port port)))

;;; Idempotent, unlike `remove-dir!` — see README.md's "Filesystem and list search".
(define (stdlib/delete-dir! dir)
  (when (path-exists? dir)
    (remove-dir! dir)))

;;; Idempotent, unlike Steel's own `delete-file!` — see README.md's "Filesystem and list search".
(define (stdlib/delete-file! path)
  (when (path-exists? path)
    (delete-file! path)))

;;; `is-dir?` drops stray files alongside a directory tree — see README.md's "Filesystem and list search".
(define (stdlib/list-subdirs dir)
  (filter (lambda (name) (is-dir? (path-join dir name)))
          (sort (map file-name (read-dir dir)) string<?)))

(define (stdlib/path-absolute? path)
  (if (equal? (current-os!) "windows")
      (or (starts-with? path "\\\\")
          (and (>= (string-length path) 3)
               (char<=? #\A (char-upcase (string-ref path 0)) #\Z)
               (equal? (string-ref path 1) #\:)
               (let ((sep (string-ref path 2)))
                 (or (equal? sep #\\) (equal? sep #\/)))))
      (starts-with? path "/")))

;; ── Git probes ───────────────────────────────────────────────────────────────

(define (stdlib/run-stdout cmd args)
  (let ([result (run-capture! cmd args)])
    (and (equal? (hash-ref result 'exit) 0) (trim (hash-ref result 'stdout)))))

(define (stdlib/git-repo?)
  (and (find-program "git")
       (equal? "true" (stdlib/run-stdout "git" '("rev-parse" "--is-inside-work-tree")))))

(define (stdlib/git-toplevel)
  (and (find-program "git")
       (stdlib/run-stdout "git" '("rev-parse" "--show-toplevel"))))

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

;; ── Picker buffer-placement actions ──────────────────────────────────────────

;;; A false/no payload skips the tab but still calls `handler` — see README.md's "Picker buffer-placement".
(define (stdlib/with-tab handler)
  (lambda (payload)
    (when payload (call! "tab-new" (focused-pane)))
    (handler payload)))

;;; Shared core for `with-vsplit`/`with-split` — see README.md's "Picker buffer-placement".
(define (stdlib/with-pane-command command handler)
  (lambda (payload)
    (if payload
        (when (call! command (focused-pane)) (handler payload))
        (handler payload))))

(define (stdlib/with-vsplit handler)
  (stdlib/with-pane-command "pane-vsplit" handler))

(define (stdlib/with-split handler)
  (stdlib/with-pane-command "pane-split" handler))

(define (stdlib/buffer-actions handler)
  (list (cons "ctrl-o" handler)
        (cons "ctrl-t" (stdlib/with-tab handler))
        (cons "ctrl-v" (stdlib/with-vsplit handler))
        (cons "ctrl-s" (stdlib/with-split handler))))

;; ── Command-argument helper ──────────────────────────────────────────────────

(define (stdlib/resolve-lang-arg pane cmd arg)
  (let ([name (if (string? arg) arg (get-buffer-option pane "language"))])
    (if (not (equal? name ""))
        name
        (begin
          (log! 'info (string-append cmd ": no language given and current buffer has no language set"))
          #f))))

;; ── Word tokenization ─────────────────────────────────────────────────────────

;;; `(split-words str word-chars)`, pulling `word-chars` from `pane`'s own setting —
;;; see README.md's "Word tokenization".
(define (stdlib/split-words pane str)
  (split-words str (get-buffer-option pane "word-chars")))

;; ── Reports ───────────────────────────────────────────────────────────────────
;; See README.md's "Reports".

(define (stdlib/report-section title items hint)
  (string-append
    title " (" (number->string (length items)) ")"
    (if (and hint (not (null? items))) (string-append ", " hint) "")
    "\n"
    (if (null? items)
        "  none\n"
        (string-join (map (lambda (item) (string-append "  " item "\n")) items) ""))))

(define (stdlib/show-report! pane name sections)
  (show-read-only-buffer! pane name (string-join sections "\n")))

;; ── Plugin config helpers ────────────────────────────────────────────────────

(define (stdlib/config-value cfg key default)
  (unless (symbol? key)
    (error (string-append "stdlib/config: key " (to-string key) " must be a symbol")))
  (if (hash-contains? cfg key) (hash-ref cfg key) default))

(define (stdlib/config-key-error plugin key problem)
  (error (string-append plugin ": '" (symbol->string key) " " problem)))

(define (stdlib/config-boolean plugin cfg key default)
  (let ([v (stdlib/config-value cfg key default)])
    (unless (boolean? v)
      (stdlib/config-key-error plugin key "must be #t or #f"))
    v))

(define (stdlib/config-string plugin cfg key default)
  (let ([v (stdlib/config-value cfg key default)])
    (unless (string? v)
      (stdlib/config-key-error plugin key "must be a string"))
    v))

(define (stdlib/config-enum plugin cfg key default allowed)
  (let ([v (stdlib/config-value cfg key default)])
    (unless (member v allowed)
      (stdlib/config-key-error
        plugin key
        (string-append
          "must be one of "
          (string-join (map (lambda (s) (string-append "'" (symbol->string s))) allowed) ", ")
          ", got " (to-string v))))
    v))

(define (stdlib/config-integer plugin cfg key default minimum)
  (unless (integer? minimum)
    (error (string-append plugin ": the minimum for '" (to-string key) " must be an integer")))
  (let ([v (stdlib/config-value cfg key default)])
    (unless (and (integer? v) (>= v minimum))
      (stdlib/config-key-error
        plugin key (string-append "must be an integer >= " (number->string minimum))))
    v))

(define (stdlib/config-list plugin cfg key default)
  (let ([v (stdlib/config-value cfg key default)])
    (unless (and (list? v) (null? (filter (lambda (x) (not (string? x))) v)))
      (stdlib/config-key-error plugin key "must be a list of strings"))
    v))

;; ── call!-able commands (public API) ────────────────────────────────────────

(define-command! "stdlib/primary-selection"
  "The primary selection in the given list, or #f."
  stdlib/primary-selection)

(define-command! "stdlib/all-single-char?"
  "#t if every selection in the given list spans a single grapheme."
  stdlib/all-single-char?)

(define-command! "stdlib/single-selection?"
  "#t if the given selection list holds exactly one selection."
  stdlib/single-selection?)

(define-command! "stdlib/cursor-char-offset"
  "0-indexed head char offset of the primary selection in the given list, or #f."
  stdlib/cursor-char-offset)

(define-command! "stdlib/find"
  "First element of the given list satisfying the given predicate, or #f."
  stdlib/find)

(define-command! "stdlib/concat"
  "The given lists joined into one list, in order."
  stdlib/concat)

(define-command! "stdlib/write-file!"
  "Write the given content to the given path, creating or truncating it."
  stdlib/write-file!)

(define-command! "stdlib/delete-dir!"
  "Recursively delete the given directory. Idempotent."
  stdlib/delete-dir!)

(define-command! "stdlib/delete-file!"
  "Delete the file at the given path. Idempotent."
  stdlib/delete-file!)

(define-command! "stdlib/list-subdirs"
  "Sorted basenames of the given directory's subdirectories."
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

(define-command! "stdlib/git-repo?"
  "#t iff the editor's cwd is inside a git work tree."
  stdlib/git-repo?)

(define-command! "stdlib/git-toplevel"
  "Absolute repo root of the editor's cwd, or #f when git is missing or cwd is not in a work tree."
  stdlib/git-toplevel)

(define-command! "stdlib/resolve-lang-arg"
  "A typed language-name argument, else pane's language, else #f after an info message."
  stdlib/resolve-lang-arg)

(define-command! "stdlib/report-section"
  "One report section: a title (count) header, then the given hint after a comma when there are items (#f for none), then one indented line per item, or none."
  stdlib/report-section)

(define-command! "stdlib/show-report!"
  "Shows the given report sections, separated by blank lines, on the given pane in the read-only buffer [name]."
  stdlib/show-report!)

(define-command! "stdlib/config-boolean"
  "A #:config hash's value for the given key, or the given default if absent; errors if it isn't #t or #f."
  stdlib/config-boolean)

(define-command! "stdlib/config-string"
  "A #:config hash's value for the given key, or the given default if absent; errors if it isn't a string."
  stdlib/config-string)

(define-command! "stdlib/config-enum"
  "A #:config hash's value for the given key, or the given default if absent; errors if it isn't one of the given allowed symbols."
  stdlib/config-enum)

(define-command! "stdlib/config-integer"
  "A #:config hash's value for the given key, or the given default if absent; errors if it isn't an integer, or is below the given minimum, an integer."
  stdlib/config-integer)

(define-command! "stdlib/config-list"
  "A #:config hash's value for the given key, or the given default if absent; errors if it isn't a list of strings."
  stdlib/config-list)

(define-command! "stdlib/with-tab"
  "Wraps the given handler: opens a new tab, then calls the handler with the picker's payload. A false payload skips opening the tab but still calls the handler."
  stdlib/with-tab)

(define-command! "stdlib/with-vsplit"
  "Wraps the given handler: splits the focused pane side by side and calls the handler in the new pane with the picker's payload. A false payload skips the split (still calls the handler); a pane too small to split skips the handler too, leaving the current pane untouched."
  stdlib/with-vsplit)

(define-command! "stdlib/with-split"
  "Wraps the given handler: splits the focused pane stacked and calls the handler in the new pane with the picker's payload. A false payload skips the split (still calls the handler); a pane too small to split skips the handler too, leaving the current pane untouched."
  stdlib/with-split)

(define-command! "stdlib/buffer-actions"
  "A picker!/live-picker! #:actions alist binding Ctrl-O/T/V/S to the given handler placed in the current pane, a new tab, a vertical split, and a horizontal split respectively."
  stdlib/buffer-actions)

(define-command! "stdlib/split-words"
  "Every word in the given string, tokenized using the given buffer's own word-chars setting — split-words with word-chars pulled from get-buffer-option automatically."
  stdlib/split-words)
