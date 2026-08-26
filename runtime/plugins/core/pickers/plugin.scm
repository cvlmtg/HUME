;;; core:pickers — fuzzy pickers for files, buffers, and git-modified files,
;;; built entirely from the public plugin API (`picker!`,
;;; `picker-source-spawn!`) — exactly what a third-party picker plugin gets.
;;; Deliberately no native (Rust) picker definitions: a fixed native set
;;; would need a Rust PR for every new finder.
;;;
;;; Depends on core:stdlib (config validation calls stdlib/config-boolean,
;;; the git probes call stdlib/git-repo?/stdlib/git-toplevel, via call!) —
;;; load it first, same as core:plum/core:lsp.

;; See "Depending on another plugin" in the user manual
;; (https://cvlmtg.github.io/HUME/plugins.html#depending-on-another-plugin)
;; for why `(declared-plugins)` is enough here.
(unless (member "core:stdlib" (declared-plugins))
  (error "core:pickers: requires core:stdlib — (declare-plugin \"core:stdlib\") or (load-plugin \"core:stdlib\") before (load-plugin \"core:pickers\")"))

;; ── Config ────────────────────────────────────────────────────────────────────
;; `(plugin-config)` only returns the real hash while this body is being
;; evaluated — read it now into a `define`, never from inside a command. See
;; README's Config table for what "untracked" controls.
(define pickers/untracked
  (call! "stdlib/config-boolean" "core:pickers" (plugin-config) "untracked" #t))

;; ── fd binary ─────────────────────────────────────────────────────────────────

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
    (call! "pickers/files-picker-with" (call! "stdlib/git-repo?") (pickers/fd-binary))))

;; ── Git-modified-files picker ────────────────────────────────────────────────

;;; NUL, `git status -z`'s entry separator/terminator.
(define pickers/nul "\x0;")

;;; `-z`'s trailing NUL means the final split fragment (and the sole fragment
;;; of an empty, clean-tree output) is always "" — filtered out.
(define (pickers/parse-git-status output)
  (map (lambda (entry) (cons entry (substring entry 3 (string-length entry))))
       (filter (lambda (s) (not (equal? s ""))) (split-many output pickers/nul))))

;;; Open the git-modified-files picker for the given absolute repo `root`.
;;; Opens the picker empty immediately, then populates it once the async
;;; `git status` completes, via `spawn-async!` rather than
;;; `picker-source-spawn!`: this picker needs an "XY "-prefixed display and a
;;; bare-path payload built together from the fully parsed output, not a
;;; streaming source's per-line display-is-payload shape. Dismissing without
;;; selecting cancels the outstanding `git status` job — no point letting it
;;; keep running once nothing can show its result.
(define (pickers/open-git-picker! root)
  (let* ([job-id #f]
         [token (picker! '()
                         (lambda (path)
                           (if path
                               (switch-to-buffer! (open-buffer! (path-join root path)))
                               (cancel-async! job-id)))
                         #:prompt "git: "
                         #:pending #t)])
    (set! job-id
      (spawn-async! "git"
                    (list "status" "--porcelain" "-z" "--no-renames"
                          (string-append "--untracked-files="
                                          (if pickers/untracked "all" "no")))
                    #f
                    (lambda (stdout stderr exit-code)
                      ;; A clean tree (exit 0, empty stdout) parses to an
                      ;; empty item list and pushes as a no-op — no
                      ;; special-casing needed, same as the sync version.
                      (if (= exit-code 0)
                          (picker-push! token (pickers/parse-git-status stdout))
                          (begin
                            ;; #:token: this picker may already be closed or
                            ;; replaced by the time a slow `git status`
                            ;; fails — closing unconditionally would tear
                            ;; down whatever picker the user has open by
                            ;; then instead of preserving the "no picker on
                            ;; failure" contract for *this* session.
                            (picker-close! #:token token)
                            (log! 'error (string-append "picker-git-modified: `git status` failed: " stderr)))))))))

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
    (call! "pickers/git-picker-with" (call! "stdlib/git-toplevel"))))

;; ── Buffers picker ────────────────────────────────────────────────────────────

;;; Display: the buffer's display-ready path when it has one — a bare name
;;; would be an ambiguous basename — else the buffer name (`*scratch*`, etc).
(define (pickers/buffer-item bid)
  (let ([path (buffer-display-path bid)])
    (cons (or path (buffer-name bid)) bid)))

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
