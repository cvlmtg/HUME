;;; core:vim-keybind — depends on core:stdlib. See README.md's "How it works".
(unless (member "core:stdlib" (declared-plugins))
  (error "core:vim-keybind: requires core:stdlib — (declare-plugin \"core:stdlib\") or (load-plugin \"core:stdlib\") before (load-plugin \"core:vim-keybind\")"))

;; No #:repeatable needed — see README's dot-repeat note.
(define-command! "vim-change-to-eol"
  "Change from the cursor to the end of the line."
  (lambda (pane) (call! "goto-line-end" pane 1 #t) (call! "change" pane)))

(define-command! "vim-change-to-eol-or-copy-line"
  "Bare C on a collapsed cursor: change to end of line (vim C). With a count, or on a real selection: copy the selection onto the line(s) below."
  ;; count 0 is "no count typed" — see README.md's "How it works".
  (lambda (pane count)
    (if (and (= count 0)
             (call! "stdlib/all-single-char?" (buffer-selections pane)))
        (call! "vim-change-to-eol" pane)
        (call! "copy-selection-on-next-line" pane count))))

(define-command! "vim-delete-to-eol"
  "Delete from the cursor to the end of the line."
  (lambda (pane) (call! "goto-line-end" pane 1 #t) (call! "delete" pane)))

;; ── Line start / end ──────────────────────────────────────────────────────────
(bind-key! 'normal "0" "goto-line-start")
(bind-key! 'normal "^" "goto-first-nonblank")
(bind-key! 'normal "$" "goto-line-end")

;; ── Flip selection ────────────────────────────────────────────────────────────
;; Vim muscle-memory alias for HUME's native Ctrl-e.
(bind-key! 'extend "o" "flip-selections")

;; ── Alternate buffer ──────────────────────────────────────────────────────────
;; Portable form of vim's Ctrl-^; see README for legacy-terminal caveat.
(bind-key! 'normal "ctrl-6" "goto-alternate-buffer")

;; ── C / D ─────────────────────────────────────────────────────────────────────
;; See README.md's "How it works".
(define cfg (plugin-config))
(define change-to-eol
  (call! "stdlib/config-enum" "core:vim-keybind" cfg "change-to-eol" 'smart '(on smart off)))
(cond
  ((equal? change-to-eol 'on)    (bind-key! 'normal "C" "vim-change-to-eol"))
  ((equal? change-to-eol 'smart) (bind-key! 'normal "C" "vim-change-to-eol-or-copy-line"))
  ((equal? change-to-eol 'off)   (begin)))
(bind-key! 'normal "D" "vim-delete-to-eol")
