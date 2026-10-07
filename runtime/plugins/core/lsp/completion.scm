;;; core:lsp/completion.scm — textDocument/completion. See docs/features.md.

(require "lib.scm")

;; ── The source ───────────────────────────────────────────────────────────────

(register-completion-source! "lsp"
  (lambda (id pane prefix)
    (lsp-request-all! pane "textDocument/completion" (lsp-position-params pane)
      (lambda (err results)
        (when err (lsp/report-error! "completion" err))
        (lsp/report-answer-errors! "completion" results)
        (completion-emit! id (lsp/answers results)))
      #:unavailable 'empty
      #:supersede "completion"))
  #:target 'buffer #:priority 10 #:resolve #t)

;; ── Trigger chars ─────────────────────────────────────────────────────────────

(lsp/setup-trigger-chars! 'completion "lsp" '() #f)
