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
