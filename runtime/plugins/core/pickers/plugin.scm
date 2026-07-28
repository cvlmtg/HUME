;;; core:pickers — fuzzy pickers for files, buffers, and git-modified files,
;;; built entirely from the public plugin API (`picker!`,
;;; `picker-source-spawn!`) — exactly what a third-party picker plugin gets.

;; ── Config ────────────────────────────────────────────────────────────────────
;; `(plugin-config)` only returns the real hash while this body is being
;; evaluated — read it now into a `define`, never from inside a command.
;; "untracked" controls whether picker-git-modified lists untracked files at
;; all (default #t). No collapsed-directory middle ground — a *file* picker
;; listing a bare directory row isn't useful, so this isn't git's own
;; three-way `--untracked-files` choice, just on/off.
(define pickers/untracked
  (let ([cfg (plugin-config)])
    (if (hash-contains? cfg "untracked") (hash-ref cfg "untracked") #t)))
(unless (boolean? pickers/untracked)
  (error "core:pickers: \"untracked\" must be #t or #f"))

;; ── Sync probe ──────────────────────────────────────────────────────────────
;; Sync spawn for fast small-output probes (git rev-parse class);
;; enumeration-scale output streams via `picker-source-spawn!` instead.
;; Unlike plum/run!, returns `#f` on nonzero exit rather than raising — a
;; failed probe (not a repo, no fd) is a normal branch here.

;;; Raw (untrimmed) stdout of `cmd args` if it spawns and exits 0, else `#f`.
;;; Piped stdin/stdout/stderr, never inherited. Stdout port grabbed BEFORE
;;; `wait`. Untrimmed because `-z`-delimited multi-entry output (e.g. `git
;;; status`) can have a leading space as *significant data* in its first
;;; entry — trimming the whole blob would eat it.
(define (pickers/run-stdout-raw cmd args)
  (let* ([base (with-stdin-piped (with-stderr-piped (with-stdout-piped (command cmd args))))]
         [spawned (spawn-process base)])
    (if (Ok? spawned)
        (let* ([child (Ok->value spawned)]
               [out (child-stdout child)])
          (close-output-port (child-stdin child))
          ;; Drain BEFORE wait — a child that fills its pipe buffer blocks on
          ;; write until read, so waiting first can deadlock past one buffer.
          (let ([output (read-port-to-string out)])
            (let ([wait-result (wait child)])
              (if (and (Ok? wait-result) (= (Ok->value wait-result) 0))
                  output
                  #f))))
        #f)))

;;; `pickers/run-stdout-raw`, trimmed — for single-value probes (git
;;; rev-parse class) where leading/trailing whitespace is never data.
(define (pickers/run-stdout cmd args)
  (let ([output (pickers/run-stdout-raw cmd args)])
    (and output (trim output))))

;;; #t iff the editor's cwd is inside a git *work tree*. Checks stdout, not
;;; just exit code: inside a bare repo `rev-parse` exits 0 but prints "false".
(define (pickers/git-repo?)
  (and (which "git")
       (equal? "true" (pickers/run-stdout "git" '("rev-parse" "--is-inside-work-tree")))))

;;; The fd binary to use, or `#f` — Debian packages it as `fdfind`.
(define (pickers/fd-binary)
  (cond [(which "fd") "fd"]
        [(which "fdfind") "fdfind"]
        [else #f]))

;;; Absolute repo root, or `#f` when git is missing or cwd is not in a work
;;; tree — `--show-toplevel` fails outside a work tree, so this doubles as
;;; the repo probe for picker-git-modified.
(define (pickers/git-toplevel)
  (and (which "git")
       (pickers/run-stdout "git" '("rev-parse" "--show-toplevel"))))

;; ── Files picker ──────────────────────────────────────────────────────────────

;;; Open an empty files picker and attach the streaming source `cmd args`
;;; (NUL-delimited output). Accept opens the file AND switches the focused
;;; pane — `open-buffer!` alone doesn't. `on-select` gets `#f` on dismissal.
(define (pickers/open-files-picker! cmd args)
  (let ([token (picker! '()
                        (lambda (path)
                          (when path
                            (switch-to-buffer! (open-buffer! path))))
                        #:prompt "files: ")])
    (picker-source-spawn! token cmd args #:nul #t)))

;;; Internal dispatch seam: probes passed in explicitly so tests can drive
;;; each branch via `call!` instead of manipulating PATH.
(define-command! "pickers/files-picker-with"
  "Internal: open the files picker for the given git/fd probe results."
  (lambda (git-repo? fd)
    (cond
      ;; Index read, no filesystem walk; -z + #:nul survives any filename.
      [git-repo?
       (pickers/open-files-picker!
        "git" '("ls-files" "-z" "--cached" "--others" "--exclude-standard"))]
      ;; --type f: a file picker lists files, not directories.
      [fd (pickers/open-files-picker! fd '("--type" "f" "-0"))]
      [else
       (error "picker-files: not inside a git repository and 'fd' is not installed — install fd (https://github.com/sharkdp/fd) to pick files outside git repos")])))

(define-command! "picker-files"
  "Fuzzy-pick a file in the current directory tree and open it."
  (lambda ()
    (call! "pickers/files-picker-with" (pickers/git-repo?) (pickers/fd-binary))))

;; ── Git-modified-files picker ────────────────────────────────────────────────

;;; NUL, `git status -z`'s entry separator/terminator.
(define pickers/nul "\x0;")

;;; Parse `git status --porcelain -z --no-renames` output into a list of
;;; (display . path) items. Each entry is "XY path" (repo-root-relative, per
;;; `man git-status`): display is the entry verbatim, path is the same string
;;; with the 3-char status prefix stripped. `-z` guarantees an unquoted path
;;; and a trailing NUL, so the final split fragment (and the sole fragment of
;;; an empty, clean-tree output) is always "" — filtered out.
(define (pickers/parse-git-status output)
  (map (lambda (entry) (cons entry (substring entry 3 (string-length entry))))
       (filter (lambda (s) (not (equal? s ""))) (split-many output pickers/nul))))

;;; Open the git-modified-files picker for the given absolute repo `root`.
;;; `on-select` resolves the chosen repo-root-relative path against `root`
;;; before opening it: `open-buffer!` resolves a relative path against the
;;; editor's cwd (`:pwd`), which only coincides with the repo root when
;;; `:pwd` *is* the repo root — from any subdirectory an unjoined path opens
;;; the wrong file.
(define (pickers/open-git-picker! root)
  (let ([output (pickers/run-stdout-raw
                  "git"
                  (list "status" "--porcelain" "-z" "--no-renames"
                        (string-append "--untracked-files="
                                        (if pickers/untracked "all" "no"))))])
    ;; `#f` (spawn/exit failure) and `""` (clean tree, valid empty output) are
    ;; distinct outcomes — collapsing them would report a broken `git status`
    ;; as "working tree is clean".
    (cond
      [(not output)
       (error "picker-git-modified: `git status` failed — check the repository state")]
      [else
       (let ([items (pickers/parse-git-status output)])
         (if (null? items)
             (error "picker-git-modified: no changes — working tree is clean")
             (picker! items
                      (lambda (path)
                        (when path
                          (switch-to-buffer! (open-buffer! (path-join root path)))))
                      #:prompt "git: ")))])))

;;; Internal dispatch seam: repo root passed in explicitly so tests can drive
;;; the not-a-repo branch via `call!` instead of manipulating the sandbox.
(define-command! "pickers/git-picker-with"
  "Internal: open the git-modified-files picker for the given repo root."
  (lambda (root)
    (if root
        (pickers/open-git-picker! root)
        (error "picker-git-modified: not inside a git repository (or 'git' is not installed)"))))

(define-command! "picker-git-modified"
  "Fuzzy-pick a file with staged or unstaged git changes and open it."
  (lambda ()
    (call! "pickers/git-picker-with" (pickers/git-toplevel))))

;; ── Buffers picker ────────────────────────────────────────────────────────────

;;; `path` relative to the editor cwd when it lies inside it, else unchanged
;;; (best-effort: a canonicalization mismatch just yields the absolute path).
(define (pickers/relativize path)
  (let* ([cwd (current-directory)]
         ;; Root is its own trailing slash — avoid building "//".
         [prefix (if (equal? cwd "/") "/" (string-append cwd "/"))])
    (if (starts-with? path prefix)
        (substring path (string-length prefix) (string-length path))
        path)))

;;; Display: the (relativized) path when the buffer has one — a bare name
;;; would be an ambiguous basename — else the buffer name (`*scratch*`, etc).
(define (pickers/buffer-item bid)
  (let ([path (buffer-path bid)])
    (cons (if path (pickers/relativize path) (buffer-name bid)) bid)))

(define-command! "picker-buffers"
  "Fuzzy-pick an open buffer and switch to it."
  (lambda ()
    (picker! (map pickers/buffer-item (buffers))
             (lambda (bid) (when bid (switch-to-buffer! bid)))
             #:prompt "buffers: ")))

;; ── Keybindings ───────────────────────────────────────────────────────────────
;; Extend mode falls through to the normal trie, so 'normal alone covers both.
(bind-key! 'normal "g f" "picker-files")
(bind-key! 'normal "g b" "picker-buffers")
(bind-key! 'normal "g m" "picker-git-modified")
