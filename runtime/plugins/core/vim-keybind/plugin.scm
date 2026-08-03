;;; core:vim-keybind

;; No #:repeatable needed — see README's dot-repeat note.
(define-command! "vim-change-to-eol"
  "Change from the cursor to the end of the line."
  (lambda () (call! "goto-line-end" 1 #t) (call! "change")))

(define-command! "vim-change-to-eol-or-copy-line"
  "Bare C on a collapsed cursor: change to end of line (vim C). With a count, or on a real selection: copy the selection onto the line(s) below."
  ;; count 0 is the dispatcher's spelling of "no count typed" — a count prefix,
  ;; even 1, is an explicit ask for the multicursor copy, so it wins over the
  ;; collapsed-cursor vim gesture and is forwarded verbatim.
  ;;
  ;; `:` invocation hands an arity-1 command's typed argument over as a string
  ;; (ArgSource::Minibuf in hume-editor/src/editor/dispatch.rs), while ordinary
  ;; key dispatch always supplies an integer — normalize the string case here
  ;; so `(= count 0)` below doesn't raise a type error. `:` with no argument at
  ;; all injects an integer 1, so the vim gesture is unreachable from the
  ;; command line regardless — consistent with "any count means copy".
  (lambda (count)
    (let ((n (if (string? count)
                 (or (string->number count)
                     (error (string-append
                             "vim-change-to-eol-or-copy-line: count must be a number, got \""
                             count "\"")))
                 count)))
      (if (and (= n 0)
               (call! "stdlib/all-single-char?" (current-selections)))
          (call! "vim-change-to-eol")
          (call! "copy-selection-on-next-line" n)))))

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
(bind-key! 'normal "ctrl-6" "goto-alternate-file")

;; ── C / D / G ─────────────────────────────────────────────────────────────────
;; change-to-eol: 'smart (default) → context-sensitive C; 'on → unconditional
;; change-to-eol; 'off → leave C at HUME's default (copy-selection-on-next-line).
(define cfg (plugin-config))
(define change-to-eol
  (if (hash-contains? cfg "change-to-eol")
      (hash-ref cfg "change-to-eol")
      'smart))
;; Check now, at load time, so a missing core:stdlib is a load error, not a
;; silent wrong-branch bug the first time C is pressed.
(when (and (equal? change-to-eol 'smart) (not (member "core:stdlib" (loaded-plugins))))
  (error "core:vim-keybind: 'smart change-to-eol requires core:stdlib — (load-plugin \"core:stdlib\") before (load-plugin \"core:vim-keybind\")"))
(cond
  ((equal? change-to-eol 'on)    (bind-key! 'normal "C" "vim-change-to-eol"))
  ((equal? change-to-eol 'smart) (bind-key! 'normal "C" "vim-change-to-eol-or-copy-line"))
  ((equal? change-to-eol 'off)   (begin))
  (else (error "core:vim-keybind: change-to-eol must be 'on, 'smart, or 'off")))
(bind-key! 'normal "D" "vim-delete-to-eol")
(bind-key! 'normal "G" "goto-last-line")
