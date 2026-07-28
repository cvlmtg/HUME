;;; core:classic-paste

;; ── Wrapper commands ──────────────────────────────────────────────────────────

(define-command! "classic-ring-after"
  "Paste the kill-ring head after the selection (same as \"kp)."
  (lambda ()
    (set-register-prefix! "k")
    (call! "paste-after")))

(define-command! "classic-ring-before"
  "Paste the kill-ring head before the selection (same as \"kP)."
  (lambda ()
    (set-register-prefix! "k")
    (call! "paste-before")))

(define-command! "classic-clipboard-after"
  "Paste the OS clipboard after the selection (same as \"cp)."
  (lambda ()
    (set-register-prefix! "c")
    (call! "paste-after")))

(define-command! "classic-clipboard-before"
  "Paste the OS clipboard before the selection (same as \"cP)."
  (lambda ()
    (set-register-prefix! "c")
    (call! "paste-before")))

;; ── Keybindings ───────────────────────────────────────────────────────────────
(bind-key! 'normal "p" "classic-ring-after")
(bind-key! 'normal "P" "classic-ring-before")

;; Ctrl+Shift+V needs the kitty keyboard protocol; see README.
(bind-key! 'normal "ctrl-v" "classic-clipboard-after")
(bind-key! 'normal "ctrl-shift-v" "classic-clipboard-before")
