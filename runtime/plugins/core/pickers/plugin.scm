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

(define (pickers/open-files-picker! cmd args)
  (let ([token (picker! '()
                        (lambda (path)
                          (when path
                            (switch-to-buffer! (open-buffer! path))))
                        #:prompt "files: ")])
    (picker-source-spawn! token cmd args #:nul #t)))

;;; Test seam — see README's "How it works".
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
                      (if (= exit-code 0)
                          (picker-push! token (pickers/parse-git-status stdout))
                          (begin
                            (picker-close! #:token token)
                            (log! 'error (string-append "picker-git-modified: `git status` failed: " stderr)))))))))

;;; Test seam — see README's "How it works".
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

;;; Display path when the buffer has one, else its name (`*scratch*`, etc).
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

(bind-key! 'normal "g f" "picker-files")
(bind-key! 'normal "g b" "picker-buffers")
(bind-key! 'normal "g m" "picker-git-modified")
