;;; core:helix-surround

;; ── Delimiter dispatch ────────────────────────────────────────────────────────

;; #f for unrecognised chars, and for the #f a cancelled read hands back, so
;; callers can skip gracefully.
(define (surround-cmd-for ch)
  (cond
    ((memv ch '(#\( #\))) "surround-paren")
    ((memv ch '(#\[ #\])) "surround-bracket")
    ((memv ch '(#\{ #\})) "surround-brace")
    ((memv ch '(#\< #\>)) "surround-angle")
    ((eqv? ch #\")         "surround-double-quote")
    ((eqv? ch #\')         "surround-single-quote")
    ((eqv? ch #\`)         "surround-backtick")
    (else #f)))

;; ── delete-surround ───────────────────────────────────────────────────────────

(define-command! "helix-delete-surround"
  "Delete the surrounding delimiter pair (md + char)."
  (lambda (pane)
    (on-next-char!
      (lambda (ch)
        (let ((cmd (surround-cmd-for ch)))
          (when cmd
            (call! cmd pane)
            (call! "delete" pane)))))))

;; ── replace-surround ─────────────────────────────────────────────────────────
;; Selects the pair, then reads the new char for `replace`; see README.

(define-command! "helix-replace-surround"
  "Replace the surrounding delimiter pair (mr + old_char + new_char)."
  (lambda (pane)
    (on-next-char!
      (lambda (ch)
        (let ((cmd (surround-cmd-for ch)))
          (when cmd
            (call! cmd pane)
            (on-next-char!
              (lambda (new)
                (when new (call! "replace" pane new))))))))))

;; ── keybindings ──────────────────────────────────────────────────────────────

(bind-key! 'normal "m s" "surround-add")
(bind-key! 'normal "m d" "helix-delete-surround")
(bind-key! 'normal "m r" "helix-replace-surround")
(unbind-key! 'normal "m w")
