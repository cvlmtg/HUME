;;; core:stdlib

;; ── Selection helpers (internal) ────────────────────────────────────────────
;;
;; Selections are opaque (anchor head primary?) triples — go through these
;; accessors, never car/cadr/caddr. Every helper passes #f straight through.

;;; Anchor (0-indexed char offset) of a single selection triple, or #f.
(define (stdlib/selection-anchor sel)
  (and sel (car sel)))

;;; Head (0-indexed char offset) of a single selection triple, or #f.
(define (stdlib/selection-head sel)
  (and sel (cadr sel)))

;;; #t if a single selection triple is the primary selection, or #f.
(define (stdlib/selection-primary? sel)
  (and sel (caddr sel)))

;;; The selection triple flagged primary in `sels`, or #f.
(define (stdlib/primary-selection sels)
  (and sels
       (let loop ((sels sels))
         (cond
           ((null? sels) #f)
           ((stdlib/selection-primary? (car sels)) (car sels))
           (else (loop (cdr sels)))))))

;;; #t if `sels` holds exactly one selection (a single cursor), or #f.
(define (stdlib/single-selection? sels)
  (and sels (= (length sels) 1)))

;;; #t if every selection in `sels` spans a single grapheme (anchor == head),
;;; or #f.
(define (stdlib/all-single-char? sels)
  (and sels
       (let loop ((sels sels))
         (cond
           ((null? sels) #t)
           ((= (stdlib/selection-anchor (car sels)) (stdlib/selection-head (car sels)))
            (loop (cdr sels)))
           (else #f)))))

;;; The char index (0-indexed) of the primary cursor's head in `sels`, or #f.
(define (stdlib/cursor-char-index sels)
  (stdlib/selection-head (stdlib/primary-selection sels)))

;; ── Filesystem + list-search helpers (internal) ─────────────────────────────
;;
;; Thin wrappers over Steel's `steel/filesystem`/`steel/ports` — the single
;; copy `core:plum` and `core:lsp` both call into via `call!` rather than
;; each carrying its own (their previous copies were byte-identical modulo
;; a plugin-name prefix).

;;; First element of `lst` satisfying `pred?`, or `#f`.
(define (stdlib/find pred? lst)
  (cond ((null? lst) #f)
        ((pred? (car lst)) (car lst))
        (else (stdlib/find pred? (cdr lst)))))

;;; Write `content` to `path`, creating or truncating it.
(define (stdlib/write-file path content)
  (let ([port (open-output-file path)])
    (write-string content port)
    (close-output-port port)))

;;; Recursively delete `dir`. Idempotent — a missing directory is not an
;;; error, unlike Steel's own `delete-directory!` — callers rely on this to
;;; clear a possibly-absent stale directory before a first-time
;;; clone/install.
(define (stdlib/delete-dir dir)
  (when (path-exists? dir)
    (delete-directory! dir)))

;;; Delete the file at `path`. Idempotent, unlike `delete-file!` — cleanup
;;; call sites must tolerate the file never having been created.
(define (stdlib/delete-file path)
  (when (path-exists? path)
    (delete-file! path)))

;;; Sorted basenames of `dir`'s subdirectories. `read-dir` returns full paths
;;; and yields every entry; the `is-dir?` test drops the stray files that sit
;;; alongside a directory tree (`.install-lock`, `.DS_Store`) and would
;;; otherwise be treated as one.
(define (stdlib/list-subdirs dir)
  (filter (lambda (name) (is-dir? (path-join dir name)))
          (sort (map file-name (read-dir dir)) string<?)))

;; ── Subprocess helper (internal) ─────────────────────────────────────────────
;;
;; `run-inline-output!` handles `#:inline-output` commands (process-group
;; safety for Ctrl+C) and `spawn-async!` handles enumeration-scale streams on
;; two Rust-side capture threads (hume-platform/src/process/job.rs) — this is
;; for everything else: a small-output subprocess run synchronously with the
;; TUI's raw mode still on. `core:plum` and `core:pickers` both call into it
;; rather than each carrying its own (their previous copies diverged on cwd
;; support, capture order, and raise-vs-#f failure policy — this folds all
;; three into one return shape and leaves the policy to the caller).

;;; Spawn `cmd`/`args` (in `cwd`, or the inherited directory when `cwd` is
;;; #f); blocks until exit. Returns (stdout stderr exit-code). A process that
;;; never produced an exit code — spawn or wait failure — comes back with
;;; exit-code #f and the reason in place of stderr, so one shape covers every
;;; outcome and callers pick their own raise-vs-#f policy.
;;;
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

;; ── Command-argument helper (internal) ───────────────────────────────────────
;;
;; `arg` is a string only when the user typed one on the `:` command line
;; (HUME's minibuffer dispatch hands a bare invocation or a keymap press an
;; integer instead — see hume-editor/src/editor/dispatch.rs's `ArgSource`
;; marshalling). Resolving "typed arg, else current buffer's language" is the
;; shared first step of every `:` command whose subject is a language name.

;;; Resolve a language-name argument for a `:` command: a typed string wins,
;;; otherwise the current buffer's language. Returns the name, or #f after
;;; logging a `cmd`-prefixed warning. `arg` is a string only when the user
;;; typed one — the minibuffer passes the default count 1 otherwise.
(define (stdlib/resolve-lang-arg cmd arg)
  (let ([name (if (string? arg) arg (buffer-language (current-buffer)))])
    (if (string? name)
        name
        (begin
          (log! 'warn (string-append cmd ": no language given and current buffer has no language set"))
          #f))))

;; ── Plugin-config helpers (internal) ────────────────────────────────────────
;;
;; `#:config` is an untyped hash — every plugin reading one needs the same
;; "key present? else default; then type-check what's there" shape, raising
;; an error naming both the plugin and the key so a bad value fails at load,
;; not wherever the untyped value happens to misbehave later. `core:git-diff`,
;; `core:pickers`, and `core:vim-keybind` all read `(plugin-config)` this way
;; (see their plugin.scm headers for the `core:stdlib` load-order dependency
;; this creates).

;;; `key`'s value in `cfg`, or `default` when the key is absent.
(define (stdlib/config-value cfg key default)
  (if (hash-contains? cfg key) (hash-ref cfg key) default))

;;; `stdlib/config-value`, rejecting a non-boolean with a `plugin`-prefixed error.
(define (stdlib/config-boolean plugin cfg key default)
  (let ([v (stdlib/config-value cfg key default)])
    (unless (boolean? v)
      (error (string-append plugin ": \"" key "\" must be #t or #f")))
    v))

;;; `stdlib/config-value`, rejecting a non-string with a `plugin`-prefixed error.
(define (stdlib/config-string plugin cfg key default)
  (let ([v (stdlib/config-value cfg key default)])
    (unless (string? v)
      (error (string-append plugin ": \"" key "\" must be a string")))
    v))

;;; `stdlib/config-value`, rejecting anything not in `allowed` (a list of
;;; symbols) with a `plugin`-prefixed error naming the allowed set and the
;;; offending value.
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
  (lambda (sels) (stdlib/all-single-char? sels)))

(define-command! "stdlib/single-selection?"
  "#t if the given selection list holds exactly one selection."
  (lambda (sels) (stdlib/single-selection? sels)))

(define-command! "stdlib/cursor-char-index"
  "0-indexed head char offset of the primary selection in the given list, or #f."
  (lambda (sels) (stdlib/cursor-char-index sels)))

(define-command! "stdlib/find"
  "First element of the given list satisfying the given predicate, or #f."
  (lambda (pred? lst) (stdlib/find pred? lst)))

(define-command! "stdlib/write-file"
  "Write the given content to the given path, creating or truncating it."
  (lambda (path content) (stdlib/write-file path content)))

(define-command! "stdlib/delete-dir"
  "Recursively delete the given directory. Idempotent."
  (lambda (dir) (stdlib/delete-dir dir)))

(define-command! "stdlib/delete-file"
  "Delete the file at the given path. Idempotent."
  (lambda (path) (stdlib/delete-file path)))

(define-command! "stdlib/list-subdirs"
  "Sorted basenames of the given directory's subdirectories."
  (lambda (dir) (stdlib/list-subdirs dir)))

(define-command! "stdlib/run"
  "Spawn the given command with the given args (in the given cwd, or the inherited directory if #f); blocks until exit. Returns (stdout stderr exit-code), with exit-code #f and the failure reason in stderr's place on spawn/wait failure."
  (lambda (cmd args cwd) (stdlib/run cmd args cwd)))

(define-command! "stdlib/resolve-lang-arg"
  "Resolve a language-name argument for a `:` command: the given string wins, else the current buffer's language, else #f after logging a warning naming the given command."
  (lambda (cmd arg) (stdlib/resolve-lang-arg cmd arg)))

(define-command! "stdlib/config-boolean"
  "The given key's value in the given #:config hash, or the given default if absent; errors (naming the given plugin) if the resolved value isn't #t or #f."
  (lambda (plugin cfg key default) (stdlib/config-boolean plugin cfg key default)))

(define-command! "stdlib/config-string"
  "The given key's value in the given #:config hash, or the given default if absent; errors (naming the given plugin) if the resolved value isn't a string."
  (lambda (plugin cfg key default) (stdlib/config-string plugin cfg key default)))

(define-command! "stdlib/config-enum"
  "The given key's value in the given #:config hash, or the given default if absent; errors (naming the given plugin) if the resolved value isn't in the given list of allowed symbols."
  (lambda (plugin cfg key default allowed) (stdlib/config-enum plugin cfg key default allowed)))
