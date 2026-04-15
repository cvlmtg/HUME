;;; helix-surround — Helix-compat md/mr surround shortcuts
;;;
;;; Adds the Helix-style two-key surround sequences on top of the built-in
;;; `ms` + char (select-surround) commands:
;;;
;;;   md + char  — delete the surrounding delimiter pair
;;;                equivalent to `ms` + char → `d`
;;;
;;;   mr + char  — select the surrounding pair, then wait for the replacement
;;;                char (dispatches `replace` with that char as pending_char)
;;;                equivalent to `ms` + char → `r` + new_char
;;;
;;; The underlying `surround-*` selection commands are Rust builtins; this
;;; plugin wires them into the two-key sequences Helix users expect.
;;;
;;; Usage in init.scm:
;;;   (load-plugin "core:helix-surround")

;; ── Delimiter dispatch ────────────────────────────────────────────────────────

;; Map a delimiter character to the appropriate surround-* command name.
;; Returns #f for unrecognised chars so callers can skip gracefully.
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
        (call-command! cmd)
        (call-command! "delete")))))

;; ── replace-surround ─────────────────────────────────────────────────────────
;; Select the surround pair with surround-*, then request a wait-char for the
;; built-in `replace` command.  The next key pressed becomes pending_char for
;; `replace`, which uses smart replacement:
;;   `(` on a `(` cursor gives `[`, `)` gives `]` (open→open, close→close).

(define-command! "helix-replace-surround"
  "Replace the surrounding delimiter pair (mr + old_char + new_char)."
  (lambda ()
    (let ((cmd (surround-cmd-for (pending-char))))
      (when cmd
        (call-command! cmd)
        (request-wait-char! "replace")))))

;; ── keybindings ──────────────────────────────────────────────────────────────

;; md + char → delete surround (any recognised delimiter)
(bind-wait-char! "normal" "m d" "helix-delete-surround")

;; mr + char → replace surround (select old pair, then wait for new char)
(bind-wait-char! "normal" "m r" "helix-replace-surround")
