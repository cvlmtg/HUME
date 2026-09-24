;;; core:pickers

(unless (member "core:stdlib" (declared-plugins))
  (error "core:pickers: requires core:stdlib — (declare-plugin \"core:stdlib\") or (load-plugin \"core:stdlib\") before (load-plugin \"core:pickers\")"))

;; ── Config ────────────────────────────────────────────────────────────────────

(define pickers/untracked
  (call! "stdlib/config-boolean" "core:pickers" (plugin-config) "untracked" #t))

;; ── fd binary ─────────────────────────────────────────────────────────────────

;;; The fd binary to use, or `#f` — Debian packages it as `fdfind`.
(define (pickers/fd-binary)
  (cond [(which "fd") "fd"]
        [(which "fdfind") "fdfind"]
        [else #f]))

;; ── Files picker ──────────────────────────────────────────────────────────────

;;; Hoisted to a name (rather than an inline lambda) so it can be passed
;;; twice: once as `on-select`, once into `stdlib/buffer-actions` — both
;;; need the identical handler.
(define (pickers/open-file! path)
  (when path
    (switch-to-buffer! (focused-pane) (open-buffer! path))))

(define (pickers/open-files-picker! pane cmd args)
  (let ([token (picker! pane '()
                        pickers/open-file!
                        #:prompt "files: "
                        #:actions (call! "stdlib/buffer-actions" pickers/open-file!))])
    (picker-source-spawn! token cmd args #:nul #t)))

;;; Test seam — see README's "How it works".
(define-command! "pickers/files-picker-with"
  "Internal: open the files picker for the given git/fd probe results."
  (lambda (pane git-repo? fd)
    (cond
      ;; Index read, no filesystem walk; -z + #:nul survives any filename.
      [git-repo?
       (pickers/open-files-picker!
        pane "git" '("ls-files" "-z" "--cached" "--others" "--exclude-standard"))]
      ;; --type f: a file picker lists files, not directories.
      [fd (pickers/open-files-picker! pane fd '("--type" "f" "-0"))]
      [else
       (error "picker-files: not inside a git repository and 'fd' is not installed — install fd (https://github.com/sharkdp/fd) to pick files outside git repos")])))

(define-command! "picker-files"
  "Fuzzy-pick a file in the current directory tree and open it."
  (lambda (pane)
    (call! "pickers/files-picker-with" pane (call! "stdlib/git-repo?") (pickers/fd-binary))))

;; ── Git-modified-files picker ────────────────────────────────────────────────

;;; NUL, `git status -z`'s entry separator/terminator.
(define pickers/nul "\x0;")

;;; `-z`'s trailing NUL means the final split fragment (and the sole fragment
;;; of an empty, clean-tree output) is always "" — filtered out.
(define (pickers/parse-git-status output)
  (map (lambda (entry) (cons entry (substring entry 3 (string-length entry))))
       (filter (lambda (s) (not (equal? s ""))) (split-many output pickers/nul))))

(define (pickers/open-git-picker! pane root)
  (let* ([job-id #f]
         ;; Hoisted to a name (see `pickers/open-file!`'s comment) so it can
         ;; be passed twice — as `on-select` and into `stdlib/buffer-actions`.
         [handler (lambda (path)
                    (if path
                        (switch-to-buffer! (focused-pane) (open-buffer! (path-join root path)))
                        (cancel-async! job-id)))]
         [token (picker! pane '()
                         handler
                         #:prompt "git: "
                         #:pending #t
                         #:actions (call! "stdlib/buffer-actions" handler))])
    (set! job-id
      (spawn-async! "git"
                    (list "status" "--porcelain" "-z" "--no-renames"
                          (string-append "--untracked-files="
                                          (if pickers/untracked "all" "no")))
                    #f
                    (lambda (stdout stderr exit-code)
                      (if (= exit-code 0)
                          (picker-push! token (pickers/parse-git-status stdout))
                          (begin
                            (picker-close! #:token token)
                            (log! 'error (string-append "picker-git-modified: `git status` failed: " stderr)))))))))

;;; Test seam — see README's "How it works".
(define-command! "pickers/git-picker-with"
  "Internal: open the git-modified-files picker for the given repo root."
  (lambda (pane root)
    (if root
        (pickers/open-git-picker! pane root)
        (error "picker-git-modified: not inside a git repository (or 'git' is not installed)"))))

(define-command! "picker-git-modified"
  "Fuzzy-pick a file with staged or unstaged git changes and open it."
  (lambda (pane)
    (call! "pickers/git-picker-with" pane (call! "stdlib/git-toplevel"))))

;; ── Buffers picker ────────────────────────────────────────────────────────────

;;; Display path when the buffer has one, else its name (`*scratch*`, etc).
;;; `(buffers)` hands each entry as a pane-less pane value — used here
;;; directly as the picker payload, same as before.
(define (pickers/buffer-item pane)
  (let ([path (buffer-display-path pane)])
    (cons (or path (buffer-name pane)) pane)))

;;; Hoisted to a name (see `pickers/open-file!`'s comment) so it can be
;;; passed twice — as `on-select` and into `stdlib/buffer-actions`. The
;;; switch itself always targets whatever pane is focused when the pick is
;;; made (`(focused-pane)`), not the pane the picker itself was opened
;;; from — the two coincide for a synchronous Enter/Ctrl-o selection, but
;;; using `(focused-pane)` here keeps this correct even if that ever
;;; changes.
(define (pickers/switch-to-buffer! target)
  (when target (switch-to-buffer! (focused-pane) target)))

(define-command! "picker-buffers"
  "Fuzzy-pick an open buffer and switch to it."
  (lambda (pane)
    (picker! pane (map pickers/buffer-item (buffers))
             pickers/switch-to-buffer!
             #:prompt "buffers: "
             #:actions (call! "stdlib/buffer-actions" pickers/switch-to-buffer!))))

;; ── Keybindings ───────────────────────────────────────────────────────────────

(bind-key! 'normal "z f" "picker-files")
(bind-key! 'normal "z b" "picker-buffers")
(bind-key! 'normal "z m" "picker-git-modified")
