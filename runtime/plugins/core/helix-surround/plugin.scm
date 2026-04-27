;;; helix-surround — Helix-compat ms/md/mr surround shortcuts
;;;
;;; Replaces HUME's built-in `ms` (select-surround) and `mw` (surround-add)
;;; bindings with the Helix layout:
;;;
;;;   ms + char  — wrap each selection with `char` (and its pair-close)
;;;                same as HUME's default `mw + char`, just at the Helix key
;;;
;;;   md + char  — delete the surrounding delimiter pair
;;;
;;;   mr + char + new_char  — replace the surrounding pair with `new_char`
;;;
;;; The select-surround commands (`surround-paren`, etc.) remain registered
;;; and reachable via the typed-command interface; only the `ms` keybinding
;;; is rerouted. `mw` is unbound while this plugin is loaded.
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
        (call! cmd)
        (call! "delete")))))

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
        (call! cmd)
        (request-wait-char! "replace")))))

;; ── keybindings ──────────────────────────────────────────────────────────────

;; ms + char → add surround (Helix layout). This overwrites HUME's default
;; `ms` sub-trie (select-surround); use the typed-command names directly if
;; you still need the selection variants.
(bind-wait-char! "normal" "m s" "surround-add")

;; md + char → delete surround (any recognised delimiter)
(bind-wait-char! "normal" "m d" "helix-delete-surround")

;; mr + char → replace surround (select old pair, then wait for new char)
(bind-wait-char! "normal" "m r" "helix-replace-surround")

;; Hide HUME's default `mw + char` (surround-add) — Helix uses `ms` for that.
(unbind-key! "normal" "m w")
