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

;; ── Path-segment validation ──────────────────────────────────────────────────

;;; Safe to use as one filesystem path segment — see README.md.
(define (stdlib/safe-path-segment? name)
  (and (string? name)
       (not (equal? name ""))
       (not (equal? name "."))
       (not (equal? name ".."))
       (not (string-contains? name "/"))
       (not (string-contains? name "\\"))
       (not (string-contains? name ":"))
       (not (string-contains? name "\""))
       (not (string-contains? name "\0"))))

;; ── Subprocess helper ────────────────────────────────────────────────────────

;;; `run-capture!` (native, `hume_platform::process::run_capture`) rather
;;; than Steel's own `spawn-process`/`wait`/`child-stdout`/`child-stderr`:
;;; that shape reads stdout to EOF, then waits, then reads stderr, which
;;; deadlocks forever on a child that fills its stderr pipe before exiting —
;;; `std::process::Command::output` drains both concurrently instead. Same
;;; `(stdout stderr exit-code)` contract this used to build by hand:
;;; `exit-code` is `#f` for a spawn failure or a signal-killed child, an int
;;; otherwise — `stdlib/run-stdout` and the git probes below rely on that.
;;; No `%` prefix, unlike most native primitives: it takes no keyword
;;; arguments to flatten, so it needs no `bootstrap.scm` wrapper of the same
;;; name minus the `%` — this `define` is that direct alias instead.
(define stdlib/run run-capture!)

;; ── Git probes ───────────────────────────────────────────────────────────────

(define (stdlib/run-stdout cmd args)
  (let ([result (stdlib/run cmd args #f)])
    (and (equal? (caddr result) 0) (trim (car result)))))

(define (stdlib/git-repo?)
  (and (which "git")
       (equal? "true" (stdlib/run-stdout "git" '("rev-parse" "--is-inside-work-tree")))))

(define (stdlib/git-toplevel)
  (and (which "git")
       (stdlib/run-stdout "git" '("rev-parse" "--show-toplevel"))))

;; ── Picker buffer-placement actions ──────────────────────────────────────────

;;; A false/no payload (an empty or not-yet-matching picker) means there is
;;; nothing to place — skip the tab and call `handler` on it directly, the
;;; same "pass #f straight through" contract every selection helper above
;;; keeps, so a handler that treats #f as its own signal (cancelling an
;;; in-flight async source, say) still runs.
(define (stdlib/with-tab handler)
  (lambda (payload)
    (when payload (call! "tab-new"))
    (handler payload)))

;;; Shared core for `with-vsplit`/`with-split`: skip `command` on a false
;;; payload (see `with-tab`), and on a truthy one, call `handler` only if
;;; `command` actually created a new pane. A too-small pane refuses the
;;; split with a status message and no other effect — matching `:split`'s
;;; own guard, which checks before opening its path argument rather than
;;; opening it in the pane that stayed put — so `handler` (and whatever it
;;; opens) is skipped too rather than silently replacing what the pane
;;; already showed. `call!` on a native command returns `#t`/`#f` for exactly
;;; this ("did the body do its job?"), so that's the success check directly —
;;; no need to infer it from a side effect like `(panes)`'s count.
(define (stdlib/with-pane-command command handler)
  (lambda (payload)
    (if payload
        (when (call! command) (handler payload))
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

(define (stdlib/resolve-lang-arg cmd arg)
  (let ([name (if (string? arg) arg (buffer-language (current-buffer)))])
    (if (string? name)
        name
        (begin
          (log! 'info (string-append cmd ": no language given and current buffer has no language set"))
          #f))))

;; ── Plugin config helpers ────────────────────────────────────────────────────

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

(define (stdlib/config-integer plugin cfg key default minimum)
  (let ([v (stdlib/config-value cfg key default)])
    (unless (and (integer? v) (or (not minimum) (>= v minimum)))
      (error (string-append plugin ": \"" key "\" must be an integer"
                             (if minimum (string-append " >= " (to-string minimum)) ""))))
    v))

(define (stdlib/config-list plugin cfg key default)
  (let ([v (stdlib/config-value cfg key default)])
    (unless (and (list? v) (null? (filter (lambda (x) (not (string? x))) v)))
      (error (string-append plugin ": \"" key "\" must be a list of strings")))
    v))

;; ── call!-able commands (public API) ────────────────────────────────────────

(define-command! "stdlib/selection-anchor"
  "Anchor char offset of the given selection triple, or #f."
  stdlib/selection-anchor)

(define-command! "stdlib/selection-head"
  "Head char offset of the given selection triple, or #f."
  stdlib/selection-head)

(define-command! "stdlib/selection-primary?"
  "#t if the given selection triple is the primary selection, or #f."
  stdlib/selection-primary?)

(define-command! "stdlib/primary-selection"
  "The primary selection triple in the given list, or #f."
  stdlib/primary-selection)

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

(define-command! "stdlib/safe-path-segment?"
  "#t iff the given name is safe to use as one filesystem path segment."
  stdlib/safe-path-segment?)

(define-command! "stdlib/run"
  "Spawn a command; blocks until exit. Returns (stdout stderr exit-code), exit-code #f on spawn/wait failure."
  stdlib/run)

(define-command! "stdlib/git-repo?"
  "#t iff the editor's cwd is inside a git work tree."
  stdlib/git-repo?)

(define-command! "stdlib/git-toplevel"
  "Absolute repo root of the editor's cwd, or #f when git is missing or cwd is not in a work tree."
  stdlib/git-toplevel)

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

(define-command! "stdlib/config-integer"
  "A #:config hash's value for the given key, or the given default if absent; errors if it isn't an integer, or is below the given minimum (#f for no minimum)."
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
