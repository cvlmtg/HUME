;;; core:pickers

;; ── Config ────────────────────────────────────────────────────────────────────

(define pickers/untracked
  (call! "stdlib/config-boolean" "core:pickers" (plugin-config) 'untracked #t))

(define pickers/git-status-args
  (list "status" "--porcelain" "-z" "--no-renames"
        (if pickers/untracked "--untracked-files=all" "--untracked-files=no")))

;; ── Shared ────────────────────────────────────────────────────────────────────

;;; See README.md's "How it works".
(define (pickers/switch-to! target)
  (when target (switch-to-buffer! (focused-pane) target)))

(define (pickers/open-file! path)
  (pickers/switch-to! (and path (open-buffer! path))))

(define (pickers/source-picker! pane prompt handler cmd args #:render [render #f])
  (picker-source-spawn! (picker! pane '()
                                 (call! "stdlib/picker-keymap" handler)
                                 #:prompt prompt
                                 #:render render)
                        cmd args #:nul #t))

;; ── Files picker ──────────────────────────────────────────────────────────────

;;; See README.md's "File source".
(define-command! "pickers/files"
  "Fuzzy-pick a file in the current directory tree and open it."
  (lambda (pane)
    (if (call! "stdlib/git-repo?")
        (pickers/source-picker! pane "files: " pickers/open-file!
                                "git" '("ls-files" "-z" "--cached" "--others" "--exclude-standard"))
        (let ([fd (or (find-program "fd") (find-program "fdfind"))])
          (if fd
              (pickers/source-picker! pane "files: " pickers/open-file! fd '("--type" "f" "-0"))
              (error "pickers/files: not inside a git repository and 'fd' is not installed — install fd (https://github.com/sharkdp/fd) to pick files outside git repos"))))))

;; ── Git-modified-files picker ────────────────────────────────────────────────

(define (pickers/status-path entry)
  (substring entry 3))

;;; See README.md's "Rows".
(define (pickers/status-span i c)
  (let ([scope (cond [(memv c '(#\A #\?)) "diff.plus"]
                     [(memv c '(#\D #\U)) "diff.minus"]
                     [(char=? c #\space) #f]
                     [else "diff.delta"])])
    (if scope
        (list (hash 'start-char i 'end-char (+ i 1) 'scope scope 'selected-scope "ui.text.focus"))
        '())))

(define (pickers/git-row entry budget)
  (hash 'text (if (< budget 4)
                  entry
                  (string-append (substring entry 0 3)
                                 (string-fit (pickers/status-path entry) (- budget 3) 'head)))
        'spans (append (pickers/status-span 0 (string-ref entry 0))
                       (pickers/status-span 1 (string-ref entry 1)))))

(define-command! "pickers/git-modified"
  "Fuzzy-pick a file with staged or unstaged git changes and open it."
  (lambda (pane)
    (let ([root (call! "stdlib/git-toplevel")])
      (if root
          (pickers/source-picker!
           pane "git: "
           (lambda (entry)
             (pickers/open-file! (and entry (path-join root (pickers/status-path entry)))))
           "git" pickers/git-status-args
           #:render (lambda (rows budget)
                      (map (lambda (row) (pickers/git-row (car row) budget)) rows)))
          (error "pickers/git-modified: not inside a git repository (or 'git' is not installed)")))))

;; ── Buffers picker ────────────────────────────────────────────────────────────

;;; See README.md's "Buffers".
(define (pickers/buffer-row row current)
  (let ([text (car row)]
        [buffer (cdr row)])
    (cond [(not (buffer-live? buffer)) text]
          [(buffer-dirty? buffer)
           (hash 'text text
                 'spans (list (hash 'start-char 0 'end-char (string-length text)
                                    'scope "diagnostic.warning.message-text"
                                    'selected-scope "diagnostic.warning.message"))
                 'selected-scope "diagnostic.warning.message")]
          [(equal? (buffer-key buffer) current)
           (hash 'text text
                 'spans (list (hash 'start-char 0 'end-char (string-length text) 'scope "markup.bold")))]
          [else text])))

(define-command! "pickers/buffers"
  "Fuzzy-pick an open buffer and switch to it."
  (lambda (pane)
    (let ([current (buffer-key pane)])
      (picker! pane (map (lambda (buffer)
                           (cons (or (buffer-display-path buffer) (buffer-name buffer)) buffer))
                         (buffers))
               (call! "stdlib/picker-keymap" pickers/switch-to!)
               #:prompt "buffers: "
               #:render (lambda (rows budget)
                          (map (lambda (row) (pickers/buffer-row row current)) rows))))))

;; ── Keybindings ───────────────────────────────────────────────────────────────

(bind-key! 'normal "z f" "pickers/files")
(bind-key! 'normal "z b" "pickers/buffers")
(bind-key! 'normal "z m" "pickers/git-modified")
