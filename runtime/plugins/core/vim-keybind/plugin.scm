;;; core:vim-keybind
;;;
;;; Depends on core:stdlib (config validation calls stdlib/config-enum via
;;; call!) — declare or load it first, same as core:plum/core:lsp.

;; See "Depending on another plugin" in the user manual
;; (https://cvlmtg.github.io/HUME/plugins.html#depending-on-another-plugin)
;; for why `(declared-plugins)` is enough here, even read at the top of the
;; plugin body — and why this does not catch an override that leaves no
;; `stdlib/config-enum` stub for the config read below.
(unless (member "core:stdlib" (declared-plugins))
  (error "core:vim-keybind: requires core:stdlib — (declare-plugin \"core:stdlib\") or (load-plugin \"core:stdlib\") before (load-plugin \"core:vim-keybind\")"))

;; No #:repeatable needed — see README's dot-repeat note.
(define-command! "vim-change-to-eol"
  "Change from the cursor to the end of the line."
  (lambda () (call! "goto-line-end" 1 #t) (call! "change")))

(define-command! "vim-change-to-eol-or-copy-line"
  "Bare C on a collapsed cursor: change to end of line (vim C). With a count, or on a real selection: copy the selection onto the line(s) below."
  ;; count 0 is the dispatcher's spelling of "no count typed" — a count prefix,
  ;; even 1, is an explicit ask for the multicursor copy, so it wins over the
  ;; collapsed-cursor vim gesture and is forwarded verbatim.
  (lambda (count)
    (if (and (= count 0)
             (call! "stdlib/all-single-char?" (current-selections)))
        (call! "vim-change-to-eol")
        (call! "copy-selection-on-next-line" count))))

(define-command! "vim-delete-to-eol"
  "Delete from the cursor to the end of the line."
  (lambda () (call! "goto-line-end" 1 #t) (call! "delete")))

;; ── Line start / end ──────────────────────────────────────────────────────────
(bind-key! 'normal "0" "goto-line-start")
(bind-key! 'normal "^" "goto-first-nonblank")
(bind-key! 'normal "$" "goto-line-end")

;; ── Flip selection ────────────────────────────────────────────────────────────
;; Vim muscle-memory alias for HUME's native Ctrl+e.
(bind-key! 'extend "o" "flip-selections")

;; ── Alternate buffer ──────────────────────────────────────────────────────────
;; Portable form of vim's Ctrl+^; see README for legacy-terminal caveat.
(bind-key! 'normal "ctrl-6" "goto-alternate-buffer")

;; ── C / D ─────────────────────────────────────────────────────────────────────
;; change-to-eol: 'smart (default) → context-sensitive C; 'on → unconditional
;; change-to-eol; 'off → leave C at HUME's default (copy-selection-on-next-line).
(define cfg (plugin-config))
(define change-to-eol
  (call! "stdlib/config-enum" "core:vim-keybind" cfg "change-to-eol" 'smart '(on smart off)))
(cond
  ((equal? change-to-eol 'on)    (bind-key! 'normal "C" "vim-change-to-eol"))
  ((equal? change-to-eol 'smart) (bind-key! 'normal "C" "vim-change-to-eol-or-copy-line"))
  ((equal? change-to-eol 'off)   (begin)))
(bind-key! 'normal "D" "vim-delete-to-eol")
