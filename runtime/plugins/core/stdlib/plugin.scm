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

(define-command! "stdlib/config-boolean"
  "The given key's value in the given #:config hash, or the given default if absent; errors (naming the given plugin) if the resolved value isn't #t or #f."
  (lambda (plugin cfg key default) (stdlib/config-boolean plugin cfg key default)))

(define-command! "stdlib/config-string"
  "The given key's value in the given #:config hash, or the given default if absent; errors (naming the given plugin) if the resolved value isn't a string."
  (lambda (plugin cfg key default) (stdlib/config-string plugin cfg key default)))

(define-command! "stdlib/config-enum"
  "The given key's value in the given #:config hash, or the given default if absent; errors (naming the given plugin) if the resolved value isn't in the given list of allowed symbols."
  (lambda (plugin cfg key default allowed) (stdlib/config-enum plugin cfg key default allowed)))
