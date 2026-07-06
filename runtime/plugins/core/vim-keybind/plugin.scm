;;; core:vim-keybind — vim muscle-memory keybindings.
;;;
;;; Restores line-motion keys vim users expect, plus C/D/G composites HUME
;;; does not bind natively. Native equivalents remain available either way:
;;;   $ ^ 0    → g l / g s / g h      (goto-line-end / -first-nonblank / -start)
;;;   Ctrl+6   → :b#                  (goto-alternate-file)
;;;   C        → ctrl-g l c           (bare cursor: change-to-EOL; a real
;;;                                    selection: shadows copy-selection-on-
;;;                                    next-line instead)
;;;   D        → ctrl-g l d
;;;   G        → g e                  (goto-last-line)
;;;
;;; Requires core:stdlib loaded first — C's selection-width check dispatches
;;; to stdlib/all-single-char? via call!:
;;;   (load-plugin "core:stdlib")
;;;   (load-plugin "core:vim-keybind")
;;;
;;; On a bare cursor, C is vim's change-to-EOL. With a real (multi-char)
;;; selection, C instead runs the shadowed copy-selection-on-next-line, so
;;; HUME's multicursor idiom stays reachable without dropping vim muscle
;;; memory for the common case. Pass #:config with "skip-shadows" to drop the
;;; vim override entirely and keep copy-selection-on-next-line unconditionally,
;;; e.g. because muscle memory for vim's C conflicts with it:
;;;   (load-plugin "core:vim-keybind" #:config (hash "skip-shadows" #t))

;; Dot-repeat needs no #:repeatable here: `change`/`delete` are natively
;; repeatable and capture the preceding goto-line-end(extend) step via the
;; shared selection-recipe accumulator on their own, regardless of whether
;; this wrapper is itself flagged repeatable.
(define-command! "vim-change-to-eol"
  "Change from the cursor to the end of the line."
  (lambda () (call! "goto-line-end" 1 #t) (call! "change")))

;; C is only vim's change-to-EOL on a bare cursor. With a real selection
;; already in place, vim has no equivalent gesture worth shadowing for, so we
;; fall back to HUME's native copy-selection-on-next-line instead of clobbering
;; it unconditionally.
(define-command! "vim-change-to-eol-or-copy-line"
  "Bare cursor: change to end of line (vim C). Real selection: copy it to the next line."
  (lambda ()
    (let ((sels (current-selections)))
      (if (call! "stdlib/all-single-char?" sels)
          (call! "vim-change-to-eol")
          (call! "copy-selection-on-next-line")))))

(define-command! "vim-delete-to-eol"
  "Delete from the cursor to the end of the line."
  (lambda () (call! "goto-line-end" 1 #t) (call! "delete")))

;; ── Line start / end ──────────────────────────────────────────────────────────
(bind-key! "normal" "0" "goto-line-start")
(bind-key! "normal" "^" "goto-first-nonblank")
(bind-key! "normal" "$" "goto-line-end")

;; ── Alternate buffer ──────────────────────────────────────────────────────────
;; Ctrl+6 is the portable form of vim's Ctrl+^: both share a keycap on US
;; layouts and emit identical bytes. With kitty keyboard protocol this arrives
;; as Char('6') + CONTROL; legacy terminals emit 0x1E which is not surfaced
;; here (users can fall back to `:e #`).
(bind-key! "normal" "ctrl-6" "goto-alternate-file")

;; ── C / D / G ─────────────────────────────────────────────────────────────────
;; C shadows the default copy-selection-on-next-line binding on a bare cursor
;; only (still reachable via `:copy-selection-on-next-line`, and via C itself
;; when a real selection is active). #:config (hash "skip-shadows" #t) skips
;; the binding entirely so the native default stays in place unconditionally.
(define cfg (plugin-config))
(unless (and (hash-contains? cfg "skip-shadows") (hash-ref cfg "skip-shadows"))
  (bind-key! "normal" "C" "vim-change-to-eol-or-copy-line"))
(bind-key! "normal" "D" "vim-delete-to-eol")
(bind-key! "normal" "G" "goto-last-line")
