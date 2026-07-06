;;; core:helix-surround

;; ── Delimiter dispatch ────────────────────────────────────────────────────────

;; #f for unrecognised chars so callers can skip gracefully.
(define (surround-cmd-for ch)
  (cond
    ((or (equal? ch "(") (equal? ch ")")) "surround-paren")
    ((or (equal? ch "[") (equal? ch "]")) "surround-bracket")
    ((or (equal? ch "{") (equal? ch "}")) "surround-brace")
    ((or (equal? ch "<") (equal? ch ">")) "surround-angle")
    ((equal? ch "\"")                     "surround-double-quote")
    ((equal? ch "'")                      "surround-single-quote")
    ((equal? ch "`")                      "surround-backtick")
    (else #f)))

;; ── delete-surround ───────────────────────────────────────────────────────────

(define-command! "helix-delete-surround"
  "Delete the surrounding delimiter pair (md + char)."
  (lambda ()
    (let ((cmd (surround-cmd-for (pending-char))))
      (when cmd
        (call! cmd)
        (call! "delete")))))

;; ── replace-surround ─────────────────────────────────────────────────────────
;; Selects the pair, then hands off to `replace`'s wait-char for its smart
;; open→open / close→close substitution — no delimiter logic duplicated here.

(define-command! "helix-replace-surround"
  "Replace the surrounding delimiter pair (mr + old_char + new_char)."
  (lambda ()
    (let ((cmd (surround-cmd-for (pending-char))))
      (when cmd
        (call! cmd)
        (request-wait-char! "replace")))))

;; ── keybindings ──────────────────────────────────────────────────────────────

(bind-wait-char! "normal" "m s" "surround-add")
(bind-wait-char! "normal" "m d" "helix-delete-surround")
(bind-wait-char! "normal" "m r" "helix-replace-surround")
(unbind-key! "normal" "m w")
