;;; core:git-diff — plugin.scm
;;;
;;; Entry point. HUME resolves and `require`s this file when the plugin
;;; loads (eager) or lazily activates (lazy) — see manifest.scm and
;;; README.md "Usage".
;;;
;;; Wires config, per-buffer state, and the git-ref fetch/diff pipeline to
;;; the buffer lifecycle hooks and the two toggle commands below —
;;; `toggle-git-signs` renders gutter `+`/`-`/`~` marks, `toggle-inline-diff`
;;; renders virtual deleted lines, word highlights, and the full-row
;;; background tint.
;;;
;;; Depends on core:stdlib (config validation calls stdlib/config-boolean,
;;; stdlib/config-string via call!) — load it first, same as core:plum/core:lsp.

(require "state.scm")
(require "diff.scm")
(require "render.scm")

;; See core:vim-keybind/plugin.scm for why this checks `(loaded-plugins)`
;; rather than `(declared-plugins)`.
(unless (member "core:stdlib" (loaded-plugins))
  (error "core:git-diff: requires core:stdlib — (load-plugin \"core:stdlib\") before (load-plugin \"core:git-diff\")"))

;; ── Config ────────────────────────────────────────────────────────────────────
;; `(plugin-config)` only returns the real hash while this body is being
;; evaluated — read it now into a `define`, never from inside a command or
;; hook handler.
(define git-diff/cfg (plugin-config))

;; Signs default on (cheap, no line-shifting side effects); inline rendering
;; defaults off (moves virtual rows into the buffer's visual flow) — see
;; README.md's Config table.
(define git-diff/signs-default (call! "stdlib/config-boolean" "core:git-diff" git-diff/cfg "signs" #t))
(define git-diff/inline-default (call! "stdlib/config-boolean" "core:git-diff" git-diff/cfg "inline" #f))

;; The config-default git ref — read and validated now so a bad config
;; value fails at load, not on the first debounced refresh. This is only
;; the *default*: `git-diff/buffer-ref` below resolves the ref actually
;; used per buffer, falling back to this when no runtime override
;; (`state.scm`'s "ref" field) is set.
(define git-diff/ref (call! "stdlib/config-string" "core:git-diff" git-diff/cfg "ref" "HEAD"))

;; Per-buffer ref resolution — a runtime override (set via an explicit-ref
;; toggle invocation, see the commands below) wins over the config default.
;; Guard-then-`hash-ref` per the repo's Steel idioms; an untracked buffer
;; has no entry at all, so it just falls through to the default.
(define (git-diff/buffer-ref bid)
  (let ([entry (git-diff/buffer-entry bid)])
    (or (and entry (hash-ref entry "ref")) git-diff/ref)))

;;; The hunks currently in state for `bid`, `'()` for an untracked buffer —
;;; used by a toggle command to paint an instant preview from whatever was
;;; last computed, before `force-refresh!` corrects it (see `run-toggle!`).
(define (git-diff/buffer-hunks bid)
  (let ([entry (git-diff/buffer-entry bid)])
    (if entry (hash-ref entry "hunks") '())))

;; ── Lifecycle ─────────────────────────────────────────────────────────────────

(register-hook! 'on-buffer-open
  (lambda (bid)
    (git-diff/init-buffer! bid git-diff/signs-default git-diff/inline-default)
    (git-diff/schedule-refresh! bid (git-diff/buffer-ref bid))))

(register-hook! 'on-text-changed
  (lambda (bid) (git-diff/schedule-refresh! bid (git-diff/buffer-ref bid))))

;; A commit, checkout, or index change while the buffer was open makes the
;; cached ref blob stale — clearing it forces the next refresh to re-fetch.
;; Cancels any fetch already in flight first (same reasoning as
;; `on-buffer-close` below): otherwise a fetch spawned just before the save
;; could land inside the debounce window and re-populate `ref-text` with
;; the pre-save blob, which `refresh!` would then treat as a valid cache
;; and never re-fetch from.
(register-hook! 'on-buffer-save
  (lambda (bid)
    (git-diff/cancel-fetch! bid)
    (git-diff/entry-set! bid "ref-text" #f)
    (git-diff/schedule-refresh! bid (git-diff/buffer-ref bid))))

(register-hook! 'on-buffer-close
  (lambda (bid)
    (git-diff/cancel-fetch! bid)
    (git-diff/remove-buffer! bid)))

;; ── Commands ──────────────────────────────────────────────────────────────────

;; Shared body for both toggles below — they differ only in which flag they
;; flip. `arg` is a string only when the user typed one on the `:` command
;; line (HUME's minibuffer dispatch hands a bare invocation or a keymap
;; press an integer instead — see hume-editor/src/editor/dispatch.rs's
;; `ArgSource` marshalling), so `(string? arg)` is the idiom to distinguish
;; "ref given" from "bare toggle", same as HUME's own
;; `lsp-install`/`lsp-stop`.
;;
;; An explicit ref is a *set*, never a flip: the rendering ends up on and
;; pointed at `arg`, even if it was already on against that same ref (which
;; then doubles as a manual re-fetch). `ref-text` must be cleared too — it
;; caches the *previous* ref's blob, and `force-refresh!` deliberately
;; preserves a valid string cache (diff.scm), so it would not re-fetch on
;; its own.
(define (git-diff/run-toggle! bid key label arg)
  (let ([enabled?
         (if (string? arg)
             (begin (git-diff/ensure-entry! bid)
                    (git-diff/entry-set! bid key #t)
                    (git-diff/entry-set! bid "ref" arg)
                    (git-diff/entry-set! bid "ref-text" #f)
                    #t)
             (git-diff/toggle-flag! bid key))])
    (if enabled?
        (begin
          ;; Paint from whatever's already in state first — a toggle
          ;; should feel instant, and `force-refresh!` no longer resets
          ;; `hunks` itself (diff.scm), so this is also what keeps state
          ;; in sync with the screen: without it, a refresh that
          ;; legitimately comes back empty would look identical to state's
          ;; untouched old value and `apply-hunks!` would skip clearing it.
          (git-diff/render-for! key bid (git-diff/buffer-hunks bid))
          ;; Then refresh for real — through neither the debounce nor a
          ;; sticky `'unavailable` cache (diff.scm's force-refresh!).
          (git-diff/force-refresh! bid (git-diff/buffer-ref bid)))
        (git-diff/render-for! key bid '()))
    (log! 'info (if enabled?
                    (string-append "git-diff: " label " on (" (git-diff/buffer-ref bid) ")")
                    (string-append "git-diff: " label " off")))))

(define-command! "toggle-git-signs"
  "Toggle gutter +/-/~ signs for the current buffer's git diff. Optional argument: a git ref to diff against, e.g. :toggle-git-signs HEAD~2 (default: the `ref` config value, shared with toggle-inline-diff)."
  (lambda (arg) (git-diff/run-toggle! (current-buffer) "signs?" "signs" arg)))

(define-command! "toggle-inline-diff"
  "Toggle inline git diff rendering (virtual deleted lines, word highlights, background tint). Optional argument: a git ref to diff against, e.g. :toggle-inline-diff HEAD~2 (default: the `ref` config value, shared with toggle-git-signs)."
  (lambda (arg) (git-diff/run-toggle! (current-buffer) "inline?" "inline diff" arg)))
