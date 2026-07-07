;;; core:vim-keybind

;; Dot-repeat needs no #:repeatable here: change/delete are natively
;; repeatable and capture the preceding goto-line-end(extend) step
;; themselves, regardless of whether this wrapper is flagged repeatable.
(define-command! "vim-change-to-eol"
  "Change from the cursor to the end of the line."
  (lambda () (call! "goto-line-end" 1 #t) (call! "change")))

;; Falls back to HUME's native copy-selection-on-next-line on a real
;; selection instead of clobbering it unconditionally.
(define-command! "vim-change-to-eol-or-copy-line"
  "Bare cursor: change to end of line (vim C). Real selection: copy it to the next line."
  (lambda ()
    (let ((sels (current-selections)))
      (if (call! "stdlib/all-single-char?" sels)
          (call! "vim-change-to-eol")
          (call! "copy-selection-on-next-line")))))

(define-command! "vim-delete-to-eol"
  "Delete from the cursor to the end of the line."
  (lambda () (call! "goto-line-end" 1 #t) (call! "delete")))

;; ── Line start / end ──────────────────────────────────────────────────────────
(bind-key! 'normal "0" "goto-line-start")
(bind-key! 'normal "^" "goto-first-nonblank")
(bind-key! 'normal "$" "goto-line-end")

;; ── Alternate buffer ──────────────────────────────────────────────────────────
;; Ctrl+6 is the portable form of vim's Ctrl+^ (same keycap/bytes on US
;; layouts). Kitty protocol delivers Char('6')+CONTROL; legacy 0x1E is not
;; surfaced here (falls back to `:e #`).
(bind-key! 'normal "ctrl-6" "goto-alternate-file")

;; ── C / D / G ─────────────────────────────────────────────────────────────────
;; change-to-eol: 'smart (default) → context-sensitive C; 'on → unconditional
;; change-to-eol; 'off → leave C at HUME's default (copy-selection-on-next-line).
(define cfg (plugin-config))
(define change-to-eol
  (if (hash-contains? cfg "change-to-eol")
      (hash-ref cfg "change-to-eol")
      'smart))
;; 'smart dispatches to stdlib/all-single-char? via call! at keypress time —
;; without core:stdlib loaded, call! would just warn and fall through, so C
;; would silently pick the wrong branch instead of failing where the mistake
;; was made. Check now, at load time, so a missing dependency is a load error.
(when (and (equal? change-to-eol 'smart) (not (member "core:stdlib" (loaded-plugins))))
  (error "core:vim-keybind: 'smart change-to-eol requires core:stdlib — (load-plugin \"core:stdlib\") before (load-plugin \"core:vim-keybind\")"))
(cond
  ((equal? change-to-eol 'on)    (bind-key! 'normal "C" "vim-change-to-eol"))
  ((equal? change-to-eol 'smart) (bind-key! 'normal "C" "vim-change-to-eol-or-copy-line"))
  ((equal? change-to-eol 'off)   (begin))
  (else (error "core:vim-keybind: change-to-eol must be 'on, 'smart, or 'off")))
(bind-key! 'normal "D" "vim-delete-to-eol")
(bind-key! 'normal "G" "goto-last-line")
