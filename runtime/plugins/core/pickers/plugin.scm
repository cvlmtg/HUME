;;; core:pickers — fuzzy pickers for files and buffers.
;;;
;;; The shipped client of the generic picker surface (docs/FUZZY-FINDERS.md
;;; B6), built entirely from the public plugin API (`picker!`,
;;; `picker-source-spawn!`) — exactly what a third-party picker plugin gets.

;; ── Sync probe ──────────────────────────────────────────────────────────────
;;
;; Q-B8 posture: sync spawn is fine for fast, small-output commands (the
;; `git rev-parse` class); enumeration-scale output always streams through
;; `picker-source-spawn!` instead. Pattern follows core:plum's `plum/run!`,
;; except this returns `#f` on a nonzero exit instead of raising — a failed
;; probe (not a repo, no fd) is a normal branch here, not an error.

;;; Trimmed stdout of `cmd args` if it spawns and exits 0, else `#f`. stdin,
;;; stdout, and stderr are all piped (and stdin closed immediately) — never
;;; inherited from the terminal, so a probe can never scribble on the TUI.
;;; The stdout port is grabbed BEFORE `wait` (plum/run!'s pinned
;;; port-lifecycle gotcha).
(define (pickers/run-stdout cmd args)
  (let* ([base (with-stdin-piped (with-stderr-piped (with-stdout-piped (command cmd args))))]
         [spawned (spawn-process base)])
    (if (Ok? spawned)
        (let* ([child (Ok->value spawned)]
               [out (child-stdout child)])
          (close-output-port (child-stdin child))
          (let ([wait-result (wait child)])
            (if (and (Ok? wait-result) (= (Ok->value wait-result) 0))
                (trim (read-port-to-string out))
                #f)))
        #f)))

;;; #t iff the editor's cwd is inside a git *work tree*. The stdout check
;;; (not exit code alone) matters: inside a bare repo or a `.git` dir,
;;; `rev-parse` exits 0 but prints "false" — and `ls-files` would be useless.
(define (pickers/git-repo?)
  (and (which "git")
       (equal? "true" (pickers/run-stdout "git" '("rev-parse" "--is-inside-work-tree")))))

;;; The fd binary to use, or `#f`. Debian/Ubuntu package the binary as
;;; `fdfind` to avoid a name clash, so probe both names.
(define (pickers/fd-binary)
  (cond [(which "fd") "fd"]
        [(which "fdfind") "fdfind"]
        [else #f]))

;; ── Files picker ──────────────────────────────────────────────────────────────

;;; Open an empty files picker and attach the streaming source `cmd args`
;;; (NUL-delimited output). Accept opens the file AND switches the focused
;;; pane to it — `open-buffer!` alone deliberately doesn't switch panes.
;;; `on-select` receives `#f` on dismissal (Esc / picker-close! / replaced).
(define (pickers/open-files-picker! cmd args)
  (let ([token (picker! '()
                        (lambda (path)
                          (when path
                            (switch-to-buffer! (open-buffer! path))))
                        #:prompt "files: ")])
    (picker-source-spawn! token cmd args #:nul #t)))

;;; Internal dispatch seam: source selection with the environment probes
;;; passed in explicitly, so tests can drive each branch hermetically via
;;; `call!` instead of manipulating PATH.
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

;; ── Buffers picker ────────────────────────────────────────────────────────────

;;; `path` relative to the editor cwd when it lies inside it, else unchanged.
;;; Best-effort: a canonicalization mismatch (e.g. macOS `/var` vs
;;; `/private/var`) just yields the absolute path instead of breaking.
(define (pickers/relativize path)
  (let ([prefix (string-append (current-directory) "/")])
    (if (starts-with? path prefix)
        (substring path (string-length prefix) (string-length path))
        path)))

;;; Display: the (relativized) path when the buffer has one — a bare
;;; `buffer-name` is an ambiguous basename (two `mod.rs` buffers are
;;; indistinguishable) — else the name (`*scratch*`, view buffers). Payload:
;;; the BufferId, opaque to Rust.
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
;; `g` trie per docs/FUZZY-FINDERS.md's trigger-prefix decision; extend mode
;; falls through to the normal trie, so 'normal alone covers both.
(bind-key! 'normal "g f" "picker-files")
(bind-key! 'normal "g b" "picker-buffers")
