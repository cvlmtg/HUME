;;; classic-paste — opt-in "GUI-style" copy/paste split
;;;
;;; Replaces the default smart-p heuristic with a predictable split:
;;;   p / P            — paste the kill-ring head (same as "kp / "kP)
;;;   Ctrl+V           — paste the OS clipboard after  (same as "cp)
;;;   Ctrl+Shift+V     — paste the OS clipboard before (same as "cP)
;;;
;;; Default HUME behavior (smart-p: clipboard unless the last command was a
;;; change/delete) is unchanged unless you load this plugin.
;;;
;;; Usage in init.scm:
;;;   (load-plugin "core:classic-paste")

;; ── Wrapper commands ──────────────────────────────────────────────────────────
;; set-register-prefix! arms a sticky register for the following (call! …):
;;   "k" → kill-ring head, "c" → OS clipboard.

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
;; p / P → kill ring (overrides the default smart-p paste-after/paste-before).
(bind-key! "normal" "p" "classic-ring-after")
(bind-key! "normal" "P" "classic-ring-before")

;; Ctrl+V / Ctrl+Shift+V → OS clipboard.
;; NOTE: Ctrl+Shift+V is only delivered under the kitty keyboard protocol. On
;; legacy terminals it is typically encoded identically to Ctrl+V, or the
;; terminal emulator intercepts it as its own paste shortcut, so it may not
;; reach HUME. Ctrl+V is delivered on both kitty and legacy encodings.
(bind-key! "normal" "ctrl-v" "classic-clipboard-after")
(bind-key! "normal" "ctrl-shift-v" "classic-clipboard-before")
