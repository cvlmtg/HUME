;;; core:lsp/completion.scm — textDocument/completion. See docs/features.md.

(require "lib.scm")

;; ── The source ───────────────────────────────────────────────────────────────

(register-completion-source! "lsp"
  (lambda (id pane prefix)
    (if (lsp/supports? pane "completionProvider")
        (lsp-request pane "textDocument/completion" (lsp-position-params pane)
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
