;;; core:stdlib

;; ── Selection helpers (internal) ────────────────────────────────────────────
;;
;; Selections are opaque (anchor head primary?) triples — go through these
;; accessors, never car/cadr/caddr. Every helper passes #f straight through.

(define (stdlib/selection-anchor sel)
  (and sel (car sel)))

(define (stdlib/selection-head sel)
  (and sel (cadr sel)))

(define (stdlib/selection-primary? sel)
  (and sel (caddr sel)))

(define (stdlib/primary-selection sels)
  (and sels
       (let loop ((sels sels))
         (cond
           ((null? sels) #f)
           ((stdlib/selection-primary? (car sels)) (car sels))
           (else (loop (cdr sels)))))))

(define (stdlib/single-selection? sels)
  (and sels (= (length sels) 1)))

(define (stdlib/all-single-char? sels)
  (and sels
       (let loop ((sels sels))
         (cond
           ((null? sels) #t)
           ((= (stdlib/selection-anchor (car sels)) (stdlib/selection-head (car sels)))
            (loop (cdr sels)))
           (else #f)))))

(define (stdlib/cursor-char-index sels)
  (stdlib/selection-head (stdlib/primary-selection sels)))

;; ── Filesystem + list-search helpers (internal) ─────────────────────────────
;;
;; Thin wrappers over Steel's `steel/filesystem`/`steel/ports` — the single
;; copy `core:plum` and `core:lsp` both call into via `call!` rather than
;; each carrying its own.

(define (stdlib/find pred? lst)
  (cond ((null? lst) #f)
        ((pred? (car lst)) (car lst))
        (else (stdlib/find pred? (cdr lst)))))

(define (stdlib/write-file path content)
  (let ([port (open-output-file path)])
    (write-string content port)
    (close-output-port port)))

;;; Idempotent, unlike Steel's own `delete-directory!` — a missing `dir` is
;;; not an error.
(define (stdlib/delete-dir dir)
  (when (path-exists? dir)
    (delete-directory! dir)))

;;; Idempotent, unlike Steel's own `delete-file!` — a missing `path` is not
;;; an error.
(define (stdlib/delete-file path)
  (when (path-exists? path)
    (delete-file! path)))

;;; `read-dir` yields every entry, including stray files that sit alongside a
;;; directory tree (`.install-lock`, `.DS_Store`) — the `is-dir?` filter
;;; drops them.
(define (stdlib/list-subdirs dir)
  (filter (lambda (name) (is-dir? (path-join dir name)))
          (sort (map file-name (read-dir dir)) string<?)))

;; ── Subprocess helper ────────────────────────────────────────────────────────
;;
;; Distinct from `run-inline-output!` (process-group Ctrl+C safety) and
;; `spawn-async!` (two Rust-side capture threads,
;; hume-platform/src/process/job.rs) — see README for which to use when.

;;; stdin is piped and closed immediately — never inherited from HUME's own
;;; terminal, or the child's reads would race the editor's key reads. Ports
;;; are grabbed before `wait` (a Steel gotcha pinned by a permanent
;;; hume-scripting test: `child-stderr` returns #f afterwards even on a piped
;;; stream) and drained stdout-then-stderr — stdout before `wait` so a large
;;; stdout stream doesn't sit in the pipe past `wait`'s own block, stderr
;;; after since a small diagnostic tail costs nothing extra once the child
;;; has already exited.
(define (stdlib/run cmd args cwd)
  (let* ([base (with-stdin-piped (with-stderr-piped (with-stdout-piped (command cmd args))))]
         [builder (if cwd (with-current-dir base cwd) base)]
         [spawned (spawn-process builder)])
    (if (Ok? spawned)
        (let* ([child (Ok->value spawned)]
               [stdout-port (child-stdout child)]
               [stderr-port (child-stderr child)])
          (close-output-port (child-stdin child))
          (let ([stdout (read-port-to-string stdout-port)])
            (let ([wait-result (wait child)])
              (if (Ok? wait-result)
                  (list stdout (read-port-to-string stderr-port) (Ok->value wait-result))
                  (list stdout (to-string (Err->value wait-result)) #f)))))
        (list "" (to-string (Err->value spawned)) #f))))

;; ── Command-argument helper ──────────────────────────────────────────────────
;;
;; `arg` is a string only when typed on the `:` command line — see
;; hume-editor/src/editor/dispatch.rs's `ArgSource` marshalling. See README
;; for the full typed-vs-injected-count contract.
(define (stdlib/resolve-lang-arg cmd arg)
  (let ([name (if (string? arg) arg (buffer-language (current-buffer)))])
    (if (string? name)
        name
        (begin
          (log! 'warn (string-append cmd ": no language given and current buffer has no language set"))
          #f))))

;; ── Plugin config helpers ────────────────────────────────────────────────────
;;
;; `#:config` is an untyped hash — every plugin reading one needs the same
;; "key present? else default; then type-check what's there" shape, raising
;; an error naming both the plugin and the key so a bad value fails at load,
;; not wherever the untyped value happens to misbehave later.

(define (stdlib/config-value cfg key default)
  (if (hash-contains? cfg key) (hash-ref cfg key) default))

(define (stdlib/config-boolean plugin cfg key default)
  (let ([v (stdlib/config-value cfg key default)])
    (unless (boolean? v)
      (error (string-append plugin ": \"" key "\" must be #t or #f")))
    v))

(define (stdlib/config-string plugin cfg key default)
  (let ([v (stdlib/config-value cfg key default)])
    (unless (string? v)
      (error (string-append plugin ": \"" key "\" must be a string")))
    v))

(define (stdlib/config-enum plugin cfg key default allowed)
  (let ([v (stdlib/config-value cfg key default)])
    (unless (member v allowed)
      (error (string-append
              plugin ": \"" key "\" must be one of "
              (string-join (map (lambda (s) (string-append "'" (symbol->string s))) allowed) ", ")
              ", got " (to-string v))))
    v))

;; ── call!-able commands (public API) ────────────────────────────────────────
;;
;; Command name and Steel binding of the same name live in separate
;; namespaces (command registry vs. module scope) — no collision between the
;; command "stdlib/all-single-char?" and the function it wraps.

(define-command! "stdlib/all-single-char?"
  "#t if every selection in the given list spans a single grapheme."
  stdlib/all-single-char?)

(define-command! "stdlib/single-selection?"
  "#t if the given selection list holds exactly one selection."
  stdlib/single-selection?)

(define-command! "stdlib/cursor-char-index"
  "0-indexed head char offset of the primary selection in the given list, or #f."
  stdlib/cursor-char-index)

(define-command! "stdlib/find"
  "First element of the given list satisfying the given predicate, or #f."
  stdlib/find)

(define-command! "stdlib/write-file"
  "Write the given content to the given path, creating or truncating it."
  stdlib/write-file)

(define-command! "stdlib/delete-dir"
  "Recursively delete the given directory. Idempotent."
  stdlib/delete-dir)

(define-command! "stdlib/delete-file"
  "Delete the file at the given path. Idempotent."
  stdlib/delete-file)

(define-command! "stdlib/list-subdirs"
  "Sorted basenames of the given directory's subdirectories."
  stdlib/list-subdirs)

(define-command! "stdlib/run"
  "Spawn a command; blocks until exit. Returns (stdout stderr exit-code), exit-code #f on spawn/wait failure."
  stdlib/run)

(define-command! "stdlib/resolve-lang-arg"
  "A typed language-name argument, else the current buffer's language, else #f after a warning."
  stdlib/resolve-lang-arg)

(define-command! "stdlib/config-boolean"
  "A #:config hash's value for the given key, or the given default if absent; errors if it isn't #t or #f."
  stdlib/config-boolean)

(define-command! "stdlib/config-string"
  "A #:config hash's value for the given key, or the given default if absent; errors if it isn't a string."
  stdlib/config-string)

(define-command! "stdlib/config-enum"
  "A #:config hash's value for the given key, or the given default if absent; errors if it isn't one of the given allowed symbols."
  stdlib/config-enum)
