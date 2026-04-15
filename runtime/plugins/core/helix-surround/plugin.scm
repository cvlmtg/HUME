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

;; ── delete-surround ──────────────────────────────────────────────────────────

(define-command! "delete-surround-paren"
  "Delete surrounding () delimiters (md( or md))."
  (lambda () (call-command! "surround-paren") (call-command! "delete")))

(define-command! "delete-surround-bracket"
  "Delete surrounding [] delimiters (md[ or md])."
  (lambda () (call-command! "surround-bracket") (call-command! "delete")))

(define-command! "delete-surround-brace"
  "Delete surrounding {} delimiters (md{ or md})."
  (lambda () (call-command! "surround-brace") (call-command! "delete")))

(define-command! "delete-surround-angle"
  "Delete surrounding <> delimiters (md< or md>)."
  (lambda () (call-command! "surround-angle") (call-command! "delete")))

(define-command! "delete-surround-double-quote"
  "Delete surrounding \"\" delimiters (md\")."
  (lambda () (call-command! "surround-double-quote") (call-command! "delete")))

(define-command! "delete-surround-single-quote"
  "Delete surrounding '' delimiters (md')."
  (lambda () (call-command! "surround-single-quote") (call-command! "delete")))

(define-command! "delete-surround-backtick"
  "Delete surrounding `` delimiters (md`)."
  (lambda () (call-command! "surround-backtick") (call-command! "delete")))

;; ── replace-surround ─────────────────────────────────────────────────────────
;; After selecting the surround pair with surround-*, request a wait-char for
;; the built-in `replace` command.  The next key pressed becomes pending_char
;; for `replace`, which uses smart replacement:
;;   `(` on a `(` cursor gives `[`, `)` gives `]` (open→open, close→close).

(define-command! "replace-surround-paren"
  "Replace surrounding () — press the new delimiter (mr( or mr))."
  (lambda () (call-command! "surround-paren") (request-wait-char! "replace")))

(define-command! "replace-surround-bracket"
  "Replace surrounding [] — press the new delimiter (mr[ or mr])."
  (lambda () (call-command! "surround-bracket") (request-wait-char! "replace")))

(define-command! "replace-surround-brace"
  "Replace surrounding {} — press the new delimiter (mr{ or mr})."
  (lambda () (call-command! "surround-brace") (request-wait-char! "replace")))

(define-command! "replace-surround-angle"
  "Replace surrounding <> — press the new delimiter (mr< or mr>)."
  (lambda () (call-command! "surround-angle") (request-wait-char! "replace")))

(define-command! "replace-surround-double-quote"
  "Replace surrounding \"\" — press the new delimiter (mr\")."
  (lambda () (call-command! "surround-double-quote") (request-wait-char! "replace")))

(define-command! "replace-surround-single-quote"
  "Replace surrounding '' — press the new delimiter (mr')."
  (lambda () (call-command! "surround-single-quote") (request-wait-char! "replace")))

(define-command! "replace-surround-backtick"
  "Replace surrounding `` — press the new delimiter (mr`)."
  (lambda () (call-command! "surround-backtick") (request-wait-char! "replace")))

;; ── keybindings ──────────────────────────────────────────────────────────────

;; md + char → delete surround
(bind-key! "normal" "md("     "delete-surround-paren")
(bind-key! "normal" "md)"     "delete-surround-paren")
(bind-key! "normal" "md["     "delete-surround-bracket")
(bind-key! "normal" "md]"     "delete-surround-bracket")
(bind-key! "normal" "md{"     "delete-surround-brace")
(bind-key! "normal" "md}"     "delete-surround-brace")
(bind-key! "normal" "md<lt>"  "delete-surround-angle")
(bind-key! "normal" "md<gt>"  "delete-surround-angle")
(bind-key! "normal" "md\""    "delete-surround-double-quote")
(bind-key! "normal" "md'"     "delete-surround-single-quote")
(bind-key! "normal" "md`"     "delete-surround-backtick")

;; mr + char → replace surround (waits for replacement char after surround-select)
(bind-key! "normal" "mr("     "replace-surround-paren")
(bind-key! "normal" "mr)"     "replace-surround-paren")
(bind-key! "normal" "mr["     "replace-surround-bracket")
(bind-key! "normal" "mr]"     "replace-surround-bracket")
(bind-key! "normal" "mr{"     "replace-surround-brace")
(bind-key! "normal" "mr}"     "replace-surround-brace")
(bind-key! "normal" "mr<lt>"  "replace-surround-angle")
(bind-key! "normal" "mr<gt>"  "replace-surround-angle")
(bind-key! "normal" "mr\""    "replace-surround-double-quote")
(bind-key! "normal" "mr'"     "replace-surround-single-quote")
(bind-key! "normal" "mr`"     "replace-surround-backtick")
