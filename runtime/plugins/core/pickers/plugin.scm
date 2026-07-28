;;; core:pickers — fuzzy pickers for files and buffers, built entirely from
;;; the public plugin API (`picker!`, `picker-source-spawn!`) — exactly what
;;; a third-party picker plugin gets.

;; ── Sync probe ──────────────────────────────────────────────────────────────
;; Sync spawn for fast small-output probes (git rev-parse class);
;; enumeration-scale output streams via `picker-source-spawn!` instead.
;; Unlike plum/run!, returns `#f` on nonzero exit rather than raising — a
;; failed probe (not a repo, no fd) is a normal branch here.

;;; Trimmed stdout of `cmd args` if it spawns and exits 0, else `#f`. Piped
;;; stdin/stdout/stderr, never inherited. Stdout port grabbed BEFORE `wait`.
(define (pickers/run-stdout cmd args)
  (let* ([base (with-stdin-piped (with-stderr-piped (with-stdout-piped (command cmd args))))]
         [spawned (spawn-process base)])
    (if (Ok? spawned)
        (let* ([child (Ok->value spawned)]
               [out (child-stdout child)])
          (close-output-port (child-stdin child))
          ;; Drain BEFORE wait — a child that fills its pipe buffer blocks on
          ;; write until read, so waiting first can deadlock past one buffer.
          (let ([output (trim (read-port-to-string out))])
            (let ([wait-result (wait child)])
              (if (and (Ok? wait-result) (= (Ok->value wait-result) 0))
                  output
                  #f))))
        #f)))

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
