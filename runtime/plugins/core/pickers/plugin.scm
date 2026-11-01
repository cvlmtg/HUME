;;; core:pickers

;; ── Config ────────────────────────────────────────────────────────────────────

(define pickers/untracked
  (call! "stdlib/config-boolean" "core:pickers" (plugin-config) 'untracked #t))

;; ── fd binary ─────────────────────────────────────────────────────────────────

;;; The fd binary to use, or `#f` — Debian packages it as `fdfind`.
(define (pickers/fd-binary)
  (cond [(find-program "fd") "fd"]
        [(find-program "fdfind") "fdfind"]
        [else #f]))

;; ── Files picker ──────────────────────────────────────────────────────────────

(define (pickers/open-file! path)
  (when path
    (switch-to-buffer! (focused-pane) (open-buffer! path))))

(define (pickers/open-files-picker! pane cmd args)
  (let ([token (picker! pane '()
                        (call! "stdlib/picker-keymap" pickers/open-file!)
                        #:prompt "files: ")])
    (picker-source-spawn! token cmd args #:nul #t)))

;;; Test seam — see README's "Design decisions".
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
       (error "pickers/files: not inside a git repository and 'fd' is not installed — install fd (https://github.com/sharkdp/fd) to pick files outside git repos")])))

(define-command! "pickers/files"
  "Fuzzy-pick a file in the current directory tree and open it."
  (lambda (pane)
    (call! "pickers/files-picker-with" pane (call! "stdlib/git-repo?") (pickers/fd-binary))))

;; ── Git-modified-files picker ────────────────────────────────────────────────

;;; NUL, `git status -z`'s entry separator/terminator.
(define pickers/nul "\x0;")

;;; `-z`'s trailing NUL means the final split fragment is always "" — see README.md's "Git-modified files".
(define (pickers/parse-git-status output)
  (map (lambda (entry) (cons entry (substring entry 3 (string-length entry))))
       (filter (lambda (s) (not (equal? s ""))) (split-many output pickers/nul))))

;;; See README.md's "Rows".
(define (pickers/status-scope c)
  (cond [(memv c '(#\A #\?)) "diff.plus"]
        [(memv c '(#\D #\U)) "diff.minus"]
        [(char=? c #\space) #f]
        [else "diff.delta"]))

(define (pickers/status-span i c)
  (let ([scope (pickers/status-scope c)])
    (if scope (list (hash 'start-char i 'end-char (+ i 1) 'scope scope)) '())))

(define (pickers/git-row row budget)
  (let ([entry (car row)])
    (hash 'text (if (< budget 4)
                    entry
                    (string-append (substring entry 0 3)
                                   (string-fit (cdr row) (- budget 3) 'head)))
          'spans (append (pickers/status-span 0 (string-ref entry 0))
                         (pickers/status-span 1 (string-ref entry 1))))))

(define (pickers/render-git-rows rows budget)
  (map (lambda (row) (pickers/git-row row budget)) rows))

(define (pickers/open-git-picker! pane root)
  (let* ([job-id #f]
         [handler (lambda (path)
                    (when path
                      (switch-to-buffer! (focused-pane) (open-buffer! (path-join root path)))))]
         [token (picker! pane '()
                         (call! "stdlib/picker-keymap" handler)
                         #:prompt "git: "
                         #:pending #t
                         #:render pickers/render-git-rows
                         #:on-close (lambda () (cancel-async! job-id)))])
    (set! job-id
      (spawn-async! "git"
                    (list "status" "--porcelain" "-z" "--no-renames"
                          (string-append "--untracked-files="
                                          (if pickers/untracked "all" "no")))
                    (lambda (result)
                      (if (equal? (hash-ref result 'exit) 0)
                          (picker-push! token (pickers/parse-git-status (hash-ref result 'stdout)))
                          (begin
                            (picker-close! token)
                            (log! 'error (string-append "pickers/git-modified: `git status` failed: "
                                                        (hash-ref result 'stderr))))))))))

;;; Test seam — see README's "Design decisions".
(define-command! "pickers/git-picker-with"
  "Internal: open the git-modified-files picker for the given repo root."
  (lambda (pane root)
    (if root
        (pickers/open-git-picker! pane root)
        (error "pickers/git-modified: not inside a git repository (or 'git' is not installed)"))))

(define-command! "pickers/git-modified"
  "Fuzzy-pick a file with staged or unstaged git changes and open it."
  (lambda (pane)
    (call! "pickers/git-picker-with" pane (call! "stdlib/git-toplevel"))))

;; ── Buffers picker ────────────────────────────────────────────────────────────

;;; Display path when the buffer has one, else its name (`*scratch*`, etc).
;;; See README.md's "Buffers".
(define (pickers/buffer-item pane)
  (let ([path (buffer-display-path pane)])
    (cons (or path (buffer-name pane)) pane)))

;;; See README.md's "Buffers".
(define (pickers/switch-to-buffer! target)
  (when target (switch-to-buffer! (focused-pane) target)))

;;; See README.md's "Buffers".
(define (pickers/buffer-row row current)
  (let* ([text (car row)]
         [end (string-length text)])
    (cond [(not (buffer-live? (cdr row))) text]
          [(buffer-dirty? (cdr row))
           (hash 'text text
                 'spans (list (hash 'start-char 0 'end-char end
                                    'scope "diagnostic.warning.message-text"
                                    'selected-scope "diagnostic.warning.message"))
                 'selected-scope "diagnostic.warning.message")]
          [(equal? (buffer-key (cdr row)) current)
           (hash 'text text
                 'spans (list (hash 'start-char 0 'end-char end 'scope "markup.bold")))]
          [else text])))

(define-command! "pickers/buffers"
  "Fuzzy-pick an open buffer and switch to it."
  (lambda (pane)
    (let ([current (buffer-key pane)])
      (picker! pane (map pickers/buffer-item (buffers))
               (call! "stdlib/picker-keymap" pickers/switch-to-buffer!)
               #:prompt "buffers: "
               #:render (lambda (rows budget)
                          (map (lambda (row) (pickers/buffer-row row current)) rows))))))

;; ── Keybindings ───────────────────────────────────────────────────────────────

(bind-key! 'normal "z f" "pickers/files")
(bind-key! 'normal "z b" "pickers/buffers")
(bind-key! 'normal "z m" "pickers/git-modified")
