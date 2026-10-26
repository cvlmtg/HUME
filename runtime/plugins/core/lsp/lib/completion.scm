;;; core:lsp/completion.scm — textDocument/completion. See docs/features.md.

(require "helpers.scm")

;; ── The source ───────────────────────────────────────────────────────────────

(define lsp/completion-key (lsp-supersede-key))

(register-completion-source! "lsp"
  (lambda (id pane prefix)
    (let ((params (lsp-position-params pane)))
      (if params
          (lsp-request-all! pane "textDocument/completion" params
            (lambda (err results)
              (when err (lsp/report-error! "completion" err))
              (lsp/report-answer-errors! "completion" results)
              (completion-emit! id (lsp/answers results)))
            #:unavailable 'empty
            #:supersede lsp/completion-key)
          (completion-emit! id '()))))
  #:target 'buffer #:priority 10 #:resolve #t)

;; ── Trigger chars ─────────────────────────────────────────────────────────────

(lsp/setup-trigger-chars! 'completion "lsp" '() #f)
