;;; core:classic-paste

;; ── Wrapper commands ──────────────────────────────────────────────────────────

(define-command! "classic-paste/ring-after"
  "Paste the kill-ring head after the selection (same as \"kp)."
  (lambda (pane)
    (set-register-prefix! "k")
    (call! "paste-after" pane)))

(define-command! "classic-paste/ring-before"
  "Paste the kill-ring head before the selection (same as \"kP)."
  (lambda (pane)
    (set-register-prefix! "k")
    (call! "paste-before" pane)))

(define-command! "classic-paste/clipboard-after"
  "Paste the OS clipboard after the selection (same as \"cp)."
  (lambda (pane)
    (set-register-prefix! "c")
    (call! "paste-after" pane)))

(define-command! "classic-paste/clipboard-before"
  "Paste the OS clipboard before the selection (same as \"cP)."
  (lambda (pane)
    (set-register-prefix! "c")
    (call! "paste-before" pane)))

;; ── Keybindings ───────────────────────────────────────────────────────────────
(bind-key! 'normal "p" "classic-paste/ring-after")
(bind-key! 'normal "P" "classic-paste/ring-before")

;; Ctrl-Shift-v needs the kitty keyboard protocol; see README.
(bind-key! 'normal "ctrl-v" "classic-paste/clipboard-after")
(bind-key! 'normal "ctrl-shift-v" "classic-paste/clipboard-before")
