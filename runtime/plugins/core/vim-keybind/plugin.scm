;;; core:vim-keybind — vim muscle-memory keybindings.
;;;
;;; Restores line-motion keys vim users expect, plus C/D/G composites HUME
;;; does not bind natively. Native equivalents remain available either way:
;;;   $ ^ 0    → g l / g s / g h      (goto-line-end / -first-nonblank / -start)
;;;   Ctrl+6   → :b#                  (goto-alternate-file)
;;;   C        → ctrl-g l c           (shadows copy-selection-on-next-line)
;;;   D        → ctrl-g l d
;;;   G        → g e                  (goto-last-line)
;;;
;;; Usage in init.scm:
;;;   (load-plugin "core:vim-keybind")

;; Dot-repeat needs no #:repeatable here: `change`/`delete` are natively
;; repeatable and capture the preceding goto-line-end(extend) step via the
;; shared selection-recipe accumulator on their own, regardless of whether
;; this wrapper is itself flagged repeatable.
(define-command! "vim-change-to-eol"
  "Change from the cursor to the end of the line."
  (lambda () (call! "goto-line-end" 1 #t) (call! "change")))

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
;; C shadows the default copy-selection-on-next-line binding (still reachable
;; via `:copy-selection-on-next-line`).
(bind-key! "normal" "C" "vim-change-to-eol")
(bind-key! "normal" "D" "vim-delete-to-eol")
(bind-key! "normal" "G" "goto-last-line")
