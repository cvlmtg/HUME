;;; core:lsp/completion.scm — textDocument/completion. See docs/features.md.

(require "lib.scm")

;; ── The source ───────────────────────────────────────────────────────────────

(register-completion-source! "lsp"
  (lambda (id pane prefix)
    (if (null? (lsp-servers pane #:feature 'completion))
        (completion-emit! id '())
        (lsp-request-all! pane "textDocument/completion" (lsp-position-params pane)
          (lambda (err results)
            (when err (lsp/report-error! "completion" err))
            (lsp/report-answer-errors! "completion" results)
            (completion-emit! id (lsp/answers results)))
          #:supersede "completion")))
  #:target 'buffer #:priority 10 #:resolve #t)

;; ── Trigger chars ─────────────────────────────────────────────────────────────

(lsp/setup-trigger-chars! 'completion "lsp" '() #f)
