;;; core:lsp/completion.scm — textDocument/completion. See docs/features.md.
;;; Never pass #:allow-stale here (unlike hover) — stale responses are
;;; auto-cancelled/dropped.

(require "lib.scm")

;; ── The source ───────────────────────────────────────────────────────────────
;;
;; This source never reads a field of the response — it hands the whole
;; thing straight to `completion-emit!`, which decodes its own
;; `isIncomplete`/`items` on the Rust side (`hume_lsp::completion_item::
;; completion_response_items`). `res` crosses as a `JsonHandle` (every
;; lsp-request response does), so this skips the ordinary Steel<->JSON
;; round trip without needing any opt-in of its own — see `JsonHandle`'s
;; own doc (`hume-scripting/src/json.rs`) for why.

(register-completion-source! "lsp"
  (lambda (id bid prefix)
    (if (lsp/supports-for-buffer? bid "completionProvider")
        (lsp-request #f "textDocument/completion" (lsp-position-params bid)
          (lambda (err res)
            (cond
              (err (lsp/report-error "completion" err)
                   (completion-emit! id '()))
              ((void? res) (completion-emit! id '()))
              (else (completion-emit! id res))))
          #:supersede "completion")
        (completion-emit! id '())))
  #:target 'buffer #:priority 10 #:resolve #t)

;; ── Trigger chars ─────────────────────────────────────────────────────────────

(lsp/setup-trigger-chars! "completionProvider" "lsp" '() #f)

;; ── Accept ────────────────────────────────────────────────────────────────────
;; No `on-completion-accept` handler here, deliberately — see docs/features.md.
